mod planner;

use planner::plan_task;
use reqwest::blocking::Client;
use retina_traits::Reasoner;
use retina_types::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;

pub struct ClaudeReasoner {
    client: Client,
    model_id: String,
    api_key: Option<String>,
    prompt_caching: ClaudePromptCaching,
    context_management: ClaudeContextManagement,
}

impl ClaudeReasoner {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            model_id: env::var("RETINA_CLAUDE_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            api_key: env::var("ANTHROPIC_API_KEY").ok(),
            prompt_caching: ClaudePromptCaching::from_env(),
            context_management: ClaudeContextManagement::from_env(),
        }
    }

    fn call_claude(&self, request: &ReasonRequest, reflection: bool) -> Result<ReasonResponse> {
        let api_key = self.api_key.clone().ok_or_else(|| {
            KernelError::Configuration("ANTHROPIC_API_KEY is not set".to_string())
        })?;

        let payload = build_payload(
            &self.model_id,
            request,
            reflection,
            &self.prompt_caching,
            &self.context_management,
        );

        let mut request_builder = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01");
        if let Some(beta_header) =
            anthropic_beta_header_value(&self.model_id, &self.context_management)
        {
            request_builder = request_builder.header("anthropic-beta", beta_header);
        }

        let response = request_builder
            .json(&payload)
            .send()
            .map_err(|error| KernelError::Reasoning(error.to_string()))?;
        let status = response.status();
        let body_text = response
            .text()
            .map_err(|error| KernelError::Reasoning(error.to_string()))?;
        if !status.is_success() {
            return Err(map_claude_error(
                status.as_u16(),
                &body_text,
                &self.model_id,
            ));
        }
        let body: ClaudeResponse = serde_json::from_str(&body_text)
            .map_err(|error| KernelError::Reasoning(error.to_string()))?;
        let text = body
            .content
            .iter()
            .find(|block| block.block_type == "text")
            .map(|block| block.text.clone())
            .ok_or_else(|| {
                KernelError::Reasoning("Claude response did not include text content".to_string())
            })?;
        let parsed = extract_json_blob(&text)?;
        let action: ClaudeAction = serde_json::from_str(&parsed).map_err(|error| {
            KernelError::Reasoning(format!("invalid Claude JSON response: {error}"))
        })?;
        let mut response = action.into_reason_response();
        response.tokens_used = body.usage.into();
        Ok(response)
    }
}

fn map_claude_error(status: u16, body: &str, model_id: &str) -> KernelError {
    if let Ok(payload) = serde_json::from_str::<ClaudeErrorResponse>(body) {
        if payload.error.error_type == "not_found_error"
            && payload.error.message.to_lowercase().contains("model:")
        {
            return KernelError::Reasoning(format!(
                "Anthropic model '{model_id}' was not found. Set RETINA_CLAUDE_MODEL to a supported model, for example 'claude-sonnet-4-20250514'."
            ));
        }
        return KernelError::Reasoning(format!(
            "Anthropic API error (status {status}): {}",
            payload.error.message
        ));
    }

    KernelError::Reasoning(format!(
        "Anthropic API error (status {status}): {}",
        body.trim()
    ))
}

impl Default for ClaudeReasoner {
    fn default() -> Self {
        Self::new()
    }
}

impl Reasoner for ClaudeReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        if let Some(response) = plan_task(
            &request.context.task,
            request.context.last_result.as_deref(),
        ) {
            return Ok(response);
        }

        self.call_claude(request, false)
    }

    fn reflect(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        self.call_claude(request, true)
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 200_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: self.prompt_caching.enabled,
            model_id: self.model_id.clone(),
        }
    }
}

fn build_payload(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
    context_management: &ClaudeContextManagement,
) -> serde_json::Value {
    let system_blocks = build_system_blocks(reflection, prompt_caching);
    let user_content = build_user_content_blocks(request);
    let mut payload = json!({
        "model": model_id,
        "max_tokens": request.max_tokens.unwrap_or(if reflection { 256 } else { 512 }),
        "system": system_blocks,
        "messages": [
            {
                "role": "user",
                "content": user_content
            }
        ]
    });

    if let Some(edits) =
        build_context_management_edits(model_id, request, reflection, context_management)
    {
        payload["context_management"] = json!({ "edits": edits });
    }

    payload
}

fn build_system_blocks(
    reflection: bool,
    prompt_caching: &ClaudePromptCaching,
) -> Vec<serde_json::Value> {
    let stable_instructions = build_stable_instructions(reflection);
    let mut blocks = vec![json!({
        "type": "text",
        "text": "Return JSON only. Do not wrap the response in markdown fences."
    })];

    let mut stable_block = json!({
        "type": "text",
        "text": stable_instructions
    });
    if prompt_caching.enabled {
        stable_block["cache_control"] = prompt_caching.cache_control_json();
    }
    blocks.push(stable_block);
    blocks
}

fn build_user_content_blocks(request: &ReasonRequest) -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "text",
            "text": build_dynamic_context_block(request)
        }),
        json!({
            "type": "text",
            "text": request.context.render()
        }),
    ]
}

fn build_stable_instructions(reflection: bool) -> String {
    format!(
        "You are the Retina agent reasoner.\n\
Reflection mode: {reflection}.\n\
Choose exactly one action and return strict JSON with these fields:\n\
- type\n\
- command\n\
- path\n\
- root\n\
- pattern\n\
- query\n\
- content\n\
- include_content\n\
- recursive\n\
- max_entries\n\
- max_results\n\
- max_bytes\n\
- max_chars\n\
- overwrite\n\
- require_approval\n\
- expect_change\n\
- note\n\
- message\n\
- task_complete\n\
- reasoning\n\
\n\
Supported action types:\n\
- run_command\n\
- inspect_path\n\
- list_directory\n\
- find_files\n\
- search_text\n\
- read_file\n\
- extract_document_text\n\
- write_file\n\
- append_file\n\
- record_note\n\
- respond\n\
\n\
Planning rules:\n\
- The harness is your body. Explore through shell actions instead of guessing.\n\
- Treat the task state artifact as the canonical compact continuity record for this task.\n\
- Use the smallest useful next step.\n\
- Prefer structured filesystem actions over shell commands when possible.\n\
- Prefer readable text sources such as .md, .txt, code, and config files when multiple candidates could answer the task.\n\
- Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
- When a prior result already includes likely candidate paths, choose the best next read or document extraction step instead of searching again.\n\
- When the last result already contains enough evidence to answer the user, respond directly instead of repeating exploration.\n\
- If multiple files match, prefer the shallowest and most human-readable candidate unless the task explicitly asks for another one.\n\
- If the user asks a question about content, gather the evidence first and then finish with respond once you can answer directly.\n\
- If the last result already gave enough evidence, do not repeat the same exploratory step.\n\
- If a request needs discovery first, choose the exploratory action and set task_complete=false.\n\
- Set task_complete=true only when the requested work is actually complete, not when you have only found a path or partial evidence.\n\
\n\
Prefer structured filesystem actions over shell commands when possible.\n\
Only use run_command for an explicit shell command or when no structured action fits.\n\
Write and append actions should normally set require_approval=true.\n\
You are allowed to explore the workspace in bounded steps.\n\
Use find_files, list_directory, search_text, and read_file to discover what you need before acting.\n\
Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
If a request needs discovery first, choose the exploratory action and set task_complete=false.\n\
Path hints like Desktop, Documents, Downloads, and ~/ refer to locations under the user's home directory."
    )
}

fn build_dynamic_context_block(request: &ReasonRequest) -> String {
    let constraints = if request.constraints.is_empty() {
        "none".to_string()
    } else {
        request
            .constraints
            .iter()
            .map(|constraint| format!("- {constraint}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "Constraints:\n{}\n\nDynamic task context follows in the next block. Use it as the mutable working set for this step.",
        constraints
    )
}

fn build_context_management_edits(
    model_id: &str,
    request: &ReasonRequest,
    reflection: bool,
    context_management: &ClaudeContextManagement,
) -> Option<Vec<serde_json::Value>> {
    let mut edits = Vec::new();

    if context_management.tool_result_clearing_enabled {
        edits.push(json!({
            "type": "clear_tool_uses_20250919",
            "trigger": {
                "type": "input_tokens",
                "value": context_management.tool_result_trigger_tokens
            },
            "clear_tool_inputs": false
        }));
    }

    if context_management.server_side_compaction_enabled
        && model_supports_server_compaction(model_id)
    {
        edits.push(json!({
            "type": "compact_20260112",
            "trigger": {
                "type": "input_tokens",
                "value": context_management.compaction_trigger_tokens
            },
            "pause_after_compaction": false,
            "instructions": build_compaction_instructions(request, reflection)
        }));
    }

    if edits.is_empty() { None } else { Some(edits) }
}

fn build_compaction_instructions(request: &ReasonRequest, reflection: bool) -> String {
    format!(
        "Write a compact continuation artifact for this Retina task. Preserve the task goal, progress, working sources, artifact references, blockers, failed paths, and next frontier. Prefer exact file paths, IDs, and evidence references over vague prose. Keep it concise and continuation-oriented. Reflection mode: {reflection}. Task: {}. Wrap the result in <summary></summary>.",
        request.context.task
    )
}

fn anthropic_beta_header_value(
    model_id: &str,
    context_management: &ClaudeContextManagement,
) -> Option<String> {
    let mut betas = Vec::new();

    if context_management.tool_result_clearing_enabled {
        betas.push("context-management-2025-06-27");
    }

    if context_management.server_side_compaction_enabled
        && model_supports_server_compaction(model_id)
    {
        betas.push("compact-2026-01-12");
    }

    if betas.is_empty() {
        None
    } else {
        Some(betas.join(","))
    }
}

fn model_supports_server_compaction(model_id: &str) -> bool {
    matches!(model_id, "claude-sonnet-4-6" | "claude-opus-4-6")
}

fn extract_json_blob(text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let mut lines = trimmed.lines();
        let _ = lines.next();
        let body = lines
            .take_while(|line| !line.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(body.trim().to_string());
    }

    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Ok(trimmed.to_string());
    }

    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            let candidate = trimmed[start..=end].trim();
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Ok(candidate.to_string());
            }
        }
    }

    Err(KernelError::Reasoning(format!(
        "Claude did not return parseable JSON. Raw response: {}",
        trimmed
    )))
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    usage: ClaudeUsage,
}

#[derive(Debug, Deserialize)]
struct ClaudeErrorResponse {
    error: ClaudeErrorBody,
}

#[derive(Debug, Deserialize)]
struct ClaudeErrorBody {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

impl From<ClaudeUsage> for TokenUsage {
    fn from(value: ClaudeUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cache_creation_input_tokens: value.cache_creation_input_tokens,
            cache_read_input_tokens: value.cache_read_input_tokens,
        }
    }
}

#[derive(Clone, Debug)]
struct ClaudePromptCaching {
    enabled: bool,
}

impl ClaudePromptCaching {
    fn from_env() -> Self {
        let enabled = env::var("RETINA_CLAUDE_PROMPT_CACHE")
            .map(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
            })
            .unwrap_or(true);
        Self { enabled }
    }

    fn cache_control_json(&self) -> serde_json::Value {
        json!({ "type": "ephemeral" })
    }
}

#[derive(Clone, Debug)]
struct ClaudeContextManagement {
    tool_result_clearing_enabled: bool,
    tool_result_trigger_tokens: u32,
    server_side_compaction_enabled: bool,
    compaction_trigger_tokens: u32,
}

impl ClaudeContextManagement {
    fn from_env() -> Self {
        Self {
            tool_result_clearing_enabled: env_flag("RETINA_CLAUDE_CONTEXT_EDITING", true),
            tool_result_trigger_tokens: env::var("RETINA_CLAUDE_TOOL_RESULT_TRIGGER_TOKENS")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(100_000),
            server_side_compaction_enabled: env_flag("RETINA_CLAUDE_SERVER_COMPACTION", true),
            compaction_trigger_tokens: env::var("RETINA_CLAUDE_COMPACTION_TRIGGER_TOKENS")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(120_000),
        }
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(default)
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaudeAction {
    #[serde(rename = "type")]
    action_type: String,
    command: Option<String>,
    path: Option<String>,
    root: Option<String>,
    pattern: Option<String>,
    query: Option<String>,
    content: Option<String>,
    include_content: Option<bool>,
    recursive: Option<bool>,
    max_entries: Option<usize>,
    max_results: Option<usize>,
    max_bytes: Option<usize>,
    max_chars: Option<usize>,
    overwrite: Option<bool>,
    require_approval: Option<bool>,
    expect_change: Option<bool>,
    note: Option<String>,
    message: Option<String>,
    task_complete: Option<bool>,
    reasoning: Option<String>,
}

impl ClaudeAction {
    fn into_reason_response(self) -> ReasonResponse {
        let action = match self.action_type.as_str() {
            "run_command" => Action::RunCommand {
                id: ActionId::new(),
                command: self.command.unwrap_or_else(|| "pwd".to_string()),
                cwd: None,
                require_approval: self.require_approval.unwrap_or(false),
                expect_change: self.expect_change.unwrap_or(false),
                state_scope: HashScope {
                    tracked_paths: Vec::new(),
                    include_working_directory: true,
                    include_last_command: true,
                },
            },
            "inspect_path" => Action::InspectPath {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                include_content: self.include_content.unwrap_or(true),
            },
            "list_directory" => Action::ListDirectory {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                recursive: self.recursive.unwrap_or(false),
                max_entries: self.max_entries.unwrap_or(100),
            },
            "find_files" => Action::FindFiles {
                id: ActionId::new(),
                root: self.root.unwrap_or_else(|| ".".to_string()).into(),
                pattern: self.pattern.unwrap_or_else(|| "*".to_string()),
                max_results: self.max_results.unwrap_or(50),
            },
            "search_text" => Action::SearchText {
                id: ActionId::new(),
                root: self.root.unwrap_or_else(|| ".".to_string()).into(),
                query: self.query.unwrap_or_default(),
                max_results: self.max_results.unwrap_or(25),
            },
            "read_file" => Action::ReadFile {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                max_bytes: self.max_bytes,
            },
            "extract_document_text" => Action::ExtractDocumentText {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                max_chars: self.max_chars,
            },
            "write_file" => Action::WriteFile {
                id: ActionId::new(),
                path: self
                    .path
                    .unwrap_or_else(|| "retina-output.txt".to_string())
                    .into(),
                content: self.content.unwrap_or_default(),
                overwrite: self.overwrite.unwrap_or(false),
                require_approval: self.require_approval.unwrap_or(true),
            },
            "append_file" => Action::AppendFile {
                id: ActionId::new(),
                path: self
                    .path
                    .unwrap_or_else(|| "retina-output.txt".to_string())
                    .into(),
                content: self.content.unwrap_or_default(),
                require_approval: self.require_approval.unwrap_or(true),
            },
            "record_note" => Action::RecordNote {
                id: ActionId::new(),
                note: self.note.unwrap_or_else(|| "No note provided".to_string()),
            },
            _ => Action::Respond {
                id: ActionId::new(),
                message: self
                    .message
                    .unwrap_or_else(|| "I need a more specific task.".to_string()),
            },
        };

        ReasonResponse {
            action,
            task_complete: self.task_complete.unwrap_or(true),
            reasoning: self.reasoning,
            tokens_used: TokenUsage::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must_some<T>(value: Option<T>, message: &str) -> T {
        value.unwrap_or_else(|| panic!("{message}"))
    }

    fn must_json(body: &str) -> String {
        extract_json_blob(body).unwrap_or_else(|_| panic!("expected JSON blob in test body"))
    }

    #[test]
    fn payload_adds_cached_system_block_when_enabled() {
        let payload = build_payload(
            "claude-sonnet-test",
            &ReasonRequest {
                context: AssembledContext {
                    identity: "Retina/test".to_string(),
                    task: "read startup.md".to_string(),
                    task_state: TaskState {
                        goal: TaskGoal {
                            objective: "read startup.md".to_string(),
                            success_criteria: Vec::new(),
                            constraints: Vec::new(),
                        },
                        progress: TaskProgress {
                            current_phase: "starting".to_string(),
                            current_step: 1,
                            max_steps: 4,
                            completed_checkpoints: Vec::new(),
                            verified_facts: Vec::new(),
                        },
                        frontier: TaskFrontier {
                            next_action_hint: None,
                            open_questions: Vec::new(),
                            blockers: Vec::new(),
                        },
                        recent_actions: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        avoid: Vec::new(),
                        compaction: None,
                    },
                    tools: Vec::new(),
                    memory_slice: Vec::new(),
                    last_result: None,
                    last_result_summary: None,
                    recent_steps: Vec::new(),
                    operator_guidance: None,
                    current_step: 1,
                    max_steps: 4,
                },
                tools: Vec::new(),
                constraints: vec!["NoNetworkShellActions".to_string()],
                max_tokens: Some(256),
            },
            false,
            &ClaudePromptCaching { enabled: true },
            &ClaudeContextManagement {
                tool_result_clearing_enabled: false,
                tool_result_trigger_tokens: 100_000,
                server_side_compaction_enabled: false,
                compaction_trigger_tokens: 120_000,
            },
        );

        let system = must_some(
            payload.get("system").and_then(serde_json::Value::as_array),
            "system blocks",
        );
        assert_eq!(system.len(), 2);
        assert_eq!(
            system[1]
                .get("cache_control")
                .and_then(|value| value.get("type"))
                .and_then(serde_json::Value::as_str),
            Some("ephemeral")
        );
    }

    #[test]
    fn fenced_json_is_unwrapped() {
        let body = must_json("```json\n{\"type\":\"respond\",\"message\":\"hi\"}\n```");
        assert_eq!(body, "{\"type\":\"respond\",\"message\":\"hi\"}");
    }

    #[test]
    fn anthropic_not_found_model_error_is_mapped_clearly() {
        let error = map_claude_error(
            404,
            r#"{"type":"error","error":{"type":"not_found_error","message":"model: bad-model"},"request_id":"req_test"}"#,
            "bad-model",
        );
        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("RETINA_CLAUDE_MODEL"));
        assert!(message.contains("bad-model"));
    }

    #[test]
    fn json_blob_is_extracted_from_prefixed_text() {
        let body = must_json(
            "Here is the JSON you requested:\n{\n  \"type\":\"respond\",\n  \"message\":\"hi\"\n}",
        );
        assert_eq!(body, "{\n  \"type\":\"respond\",\n  \"message\":\"hi\"\n}");
    }

    #[test]
    fn claude_usage_maps_cache_token_fields() {
        let usage = ClaudeUsage {
            input_tokens: 100,
            output_tokens: 20,
            cache_creation_input_tokens: 80,
            cache_read_input_tokens: 60,
        };
        let tokens: TokenUsage = usage.into();
        assert_eq!(tokens.input_tokens, 100);
        assert_eq!(tokens.output_tokens, 20);
        assert_eq!(tokens.cache_creation_input_tokens, 80);
        assert_eq!(tokens.cache_read_input_tokens, 60);
    }

    #[test]
    fn payload_adds_context_management_for_supported_model() {
        let payload = build_payload(
            "claude-sonnet-4-6",
            &ReasonRequest {
                context: AssembledContext {
                    identity: "Retina/test".to_string(),
                    task: "read startup.md".to_string(),
                    task_state: TaskState::default(),
                    tools: Vec::new(),
                    memory_slice: Vec::new(),
                    last_result: None,
                    last_result_summary: None,
                    recent_steps: Vec::new(),
                    operator_guidance: None,
                    current_step: 1,
                    max_steps: 4,
                },
                tools: Vec::new(),
                constraints: Vec::new(),
                max_tokens: Some(256),
            },
            false,
            &ClaudePromptCaching { enabled: true },
            &ClaudeContextManagement {
                tool_result_clearing_enabled: true,
                tool_result_trigger_tokens: 90_000,
                server_side_compaction_enabled: true,
                compaction_trigger_tokens: 120_000,
            },
        );

        let edits = must_some(
            payload
                .get("context_management")
                .and_then(|value| value.get("edits"))
                .and_then(serde_json::Value::as_array),
            "context management edits",
        );
        assert_eq!(edits.len(), 2);
        assert_eq!(
            edits[0].get("type").and_then(serde_json::Value::as_str),
            Some("clear_tool_uses_20250919")
        );
        assert_eq!(
            edits[1].get("type").and_then(serde_json::Value::as_str),
            Some("compact_20260112")
        );
    }

    #[test]
    fn beta_header_combines_context_management_features() {
        let header = anthropic_beta_header_value(
            "claude-sonnet-4-6",
            &ClaudeContextManagement {
                tool_result_clearing_enabled: true,
                tool_result_trigger_tokens: 100_000,
                server_side_compaction_enabled: true,
                compaction_trigger_tokens: 120_000,
            },
        );
        assert_eq!(
            header.as_deref(),
            Some("context-management-2025-06-27,compact-2026-01-12")
        );
    }
}
