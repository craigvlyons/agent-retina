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
}

impl ClaudeReasoner {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            model_id: env::var("RETINA_CLAUDE_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            api_key: env::var("ANTHROPIC_API_KEY").ok(),
        }
    }

    fn call_claude(&self, request: &ReasonRequest, reflection: bool) -> Result<ReasonResponse> {
        let api_key = self.api_key.clone().ok_or_else(|| {
            KernelError::Configuration("ANTHROPIC_API_KEY is not set".to_string())
        })?;

        let prompt = build_prompt(request, reflection);
        let payload = json!({
            "model": self.model_id,
            "max_tokens": request.max_tokens.unwrap_or(if reflection { 256 } else { 512 }),
            "system": "Return JSON only. Do not wrap the response in markdown fences.",
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        });

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
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
        Ok(action.into_reason_response())
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
            supports_caching: false,
            model_id: self.model_id.clone(),
        }
    }
}

fn build_prompt(request: &ReasonRequest, reflection: bool) -> String {
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
Constraints:\n\
{}\n\
\n\
Prefer structured filesystem actions over shell commands when possible.\n\
Only use run_command for an explicit shell command or when no structured action fits.\n\
Write and append actions should normally set require_approval=true.\n\
You are allowed to explore the workspace in bounded steps.\n\
Use find_files, list_directory, search_text, and read_file to discover what you need before acting.\n\
Use extract_document_text for PDFs and other document formats when reading raw bytes would be unhelpful.\n\
If a request needs discovery first, choose the exploratory action and set task_complete=false.\n\
Path hints like Desktop, Documents, Downloads, and ~/ refer to locations under the user's home directory.\n\
\n\
Context:\n{}",
        constraints,
        request.context.render()
    )
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

    #[test]
    fn fenced_json_is_unwrapped() {
        let body =
            extract_json_blob("```json\n{\"type\":\"respond\",\"message\":\"hi\"}\n```").unwrap();
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
        let body = extract_json_blob(
            "Here is the JSON you requested:\n{\n  \"type\":\"respond\",\n  \"message\":\"hi\"\n}",
        )
        .unwrap();
        assert_eq!(body, "{\n  \"type\":\"respond\",\n  \"message\":\"hi\"\n}");
    }
}
