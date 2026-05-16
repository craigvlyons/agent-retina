// File boundary: keep lib.rs focused on reasoner wiring and top-level provider
// orchestration. Move payload, config, planner, and parsing logic into modules.
mod config;
mod payload;
mod planner;
mod response;

use config::{ClaudeContextManagement, ClaudePromptCaching, anthropic_beta_header_value};
use payload::build_payload;
use planner::plan_task;
use reqwest::blocking::Client;
use response::{ClaudeAction, ClaudeResponse, extract_json_blob, map_claude_error};
use retina_traits::Reasoner;
use retina_types::*;
use std::env;

#[derive(Clone, Debug)]
pub struct ClaudeRuntimeConfigSnapshot {
    pub model_id: String,
    pub prompt_caching_enabled: bool,
    pub context_editing_enabled: bool,
    pub tool_result_trigger_tokens: u32,
    pub server_side_compaction_requested: bool,
    pub server_side_compaction_supported: bool,
    pub server_side_compaction_effective: bool,
    pub compaction_trigger_tokens: u32,
}

pub struct ClaudeReasoner {
    client: Client,
    model_id: String,
    api_key: Option<String>,
    prompt_caching: ClaudePromptCaching,
    context_management: ClaudeContextManagement,
}

impl ClaudeReasoner {
    pub fn new() -> Self {
        let model_id = env::var("RETINA_CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
        Self::with_model(model_id)
    }

    pub fn with_model(model_id: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            model_id: model_id.into(),
            api_key: env::var("ANTHROPIC_API_KEY").ok(),
            prompt_caching: ClaudePromptCaching::from_env(),
            context_management: ClaudeContextManagement::from_env(),
        }
    }

    pub fn runtime_config_snapshot() -> ClaudeRuntimeConfigSnapshot {
        let model_id = env::var("RETINA_CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
        let prompt_caching = ClaudePromptCaching::from_env();
        let context_management = ClaudeContextManagement::from_env();
        let server_side_compaction_supported = config::model_supports_server_compaction(&model_id);
        ClaudeRuntimeConfigSnapshot {
            model_id,
            prompt_caching_enabled: prompt_caching.enabled,
            context_editing_enabled: context_management.tool_result_clearing_enabled,
            tool_result_trigger_tokens: context_management.tool_result_trigger_tokens,
            server_side_compaction_requested: context_management.server_side_compaction_enabled,
            server_side_compaction_supported,
            server_side_compaction_effective: context_management.server_side_compaction_enabled
                && server_side_compaction_supported,
            compaction_trigger_tokens: context_management.compaction_trigger_tokens,
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
        let mut response = action.into_reason_response()?;
        response.tokens_used = body.usage.into();
        Ok(response)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::build_stable_instructions;
    use crate::response::ClaudeUsage;

    fn must<T, E: std::fmt::Display>(value: std::result::Result<T, E>) -> T {
        value.unwrap_or_else(|error| panic!("test operation failed: {error}"))
    }

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
                            constraints: Vec::new(),
                        },
                        progress: TaskProgress {
                            current_phase: "starting".to_string(),
                            current_step: 1,
                            max_steps: 4,
                            completed_checkpoints: Vec::new(),
                            verified_facts: Vec::new(),
                            output_written: false,
                            output_verified: false,
                        },
                        recent_actions: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        compaction: None,
                    },
                    recent_context: None,
                    tools: Vec::new(),
                    memory_slice: Vec::new(),
                    last_result: None,
                    operator_guidance: None,
                    current_step: 1,
                    max_steps: 4,
                },
                tools: Vec::new(),
                constraints: vec!["DeleteOrKillRequireApproval".to_string()],
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
    fn json_blob_is_extracted_without_trailing_prose() {
        let body = must_json(
            "Let me do that.\n{\"type\":\"respond\",\"message\":\"hi\"}\nI am done now.",
        );
        assert_eq!(body, "{\"type\":\"respond\",\"message\":\"hi\"}");
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
                    recent_context: None,
                    tools: Vec::new(),
                    memory_slice: Vec::new(),
                    last_result: None,
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

    #[test]
    fn stable_instructions_prefer_best_verifiable_step_over_timid_discovery_bias() {
        let instructions = build_stable_instructions(false, &[]);
        assert!(instructions.contains(
            "Choose one concrete next step that advances the task or reduces uncertainty."
        ));
        assert!(instructions.contains("the harness can verify the change"));
        assert!(instructions.contains("In reflection mode, do something materially different or report the grounded blocker/current status."));
        assert!(instructions.contains("keep root as a real directory path"));
        assert!(
            instructions
                .contains("For terminal or system-control tasks, use command output as evidence")
        );
        assert!(instructions.contains("prefer extractive facts over interpretation"));
        assert!(instructions.contains("Do not invent action items"));
        assert!(instructions.contains("NO_SPEECH"));
        assert!(instructions.contains("Do not add generated dates"));
        assert!(instructions.contains("prefer MCP tools over ad-hoc shell web scraping"));
        assert!(instructions.contains("time-sensitive web tasks"));
        assert!(instructions.contains("generic city attractions"));
        assert!(instructions.contains("Do not mention attractions, neighborhoods, sports teams, venues, or activity categories"));
        assert!(instructions.contains("MCP tool identifiers and MCP locators are not files"));
        assert!(instructions.contains("input validation error"));
        assert!(instructions.contains(
            "Do not create or modify files unless the user asked for a saved artifact"
        ));
        assert!(!instructions.contains("best next verifiable step"));
        assert!(!instructions.contains(
            "Prefer direct interaction with the named path, file, command, or system target"
        ));
        assert!(!instructions.contains("prefer write_file or append_file"));
        assert!(!instructions.contains("overwrite=true"));
        assert!(!instructions.contains("pending deliverable or target output path"));
    }

    #[test]
    fn stable_instructions_prefer_concrete_mcp_tools_over_generic_wrapper() {
        let instructions = build_stable_instructions(
            false,
            &[ToolDescriptor {
                name: "mcp__brave__brave_web_search".to_string(),
                description: "Search the web".to_string(),
                source: ToolSourceKind::McpServer,
                concurrency: ToolConcurrencyClass::Streaming,
                approval: ToolApprovalPolicy::None,
                required_authority: vec!["mcp".to_string()],
                streaming: true,
            }],
        );
        assert!(instructions.contains("Use the concrete MCP tool action types"));
        assert!(instructions.contains(
            "\"type\": \"mcp__brave__brave_web_search\""
        ));
        assert!(instructions.contains(
            "Do not start with run_command, curl, or ad-hoc web scraping"
        ));
        assert!(!instructions.contains(
            "Use mcp_call with `server`, `tool`, and `input_json`"
        ));
    }

    #[test]
    fn concrete_mcp_tool_name_maps_into_call_mcp_tool_action() {
        let action: ClaudeAction = serde_json::from_str(
            r#"{
                "type": "mcp__brave__brave_web_search",
                "input_json": {"query":"date ideas in colorado springs"}
            }"#,
        )
        .unwrap_or_else(|error| panic!("failed to parse ClaudeAction: {error}"));

        let response = action
            .into_reason_response()
            .unwrap_or_else(|error| panic!("failed to convert ClaudeAction: {error}"));
        let Action::CallMcpTool {
            server,
            tool,
            resolved_tool_name,
            ..
        } = response.action
        else {
            panic!("expected CallMcpTool action");
        };
        assert_eq!(server, "brave");
        assert_eq!(tool, "brave_web_search");
        assert_eq!(
            resolved_tool_name.as_deref(),
            Some("mcp__brave__brave_web_search")
        );
    }

    #[test]
    fn answer_task_payload_no_longer_serializes_required_input_hints() {
        let request = ReasonRequest {
            context: AssembledContext {
                identity: "retina".to_string(),
                task: "read the Patent Center.pdf on Desktop and tell me what it's about"
                    .to_string(),
                task_state: TaskState {
                    goal: TaskGoal {
                        objective:
                            "read the Patent Center.pdf on Desktop and tell me what it's about"
                                .to_string(),
                        constraints: Vec::new(),
                    },
                    progress: TaskProgress {
                        current_phase: "planning".to_string(),
                        current_step: 1,
                        max_steps: 6,
                        completed_checkpoints: Vec::new(),
                        verified_facts: Vec::new(),
                        output_written: false,
                        output_verified: false,
                    },
                    recent_actions: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    compaction: None,
                },
                recent_context: None,
                tools: Vec::new(),
                memory_slice: Vec::new(),
                last_result: None,
                operator_guidance: None,
                current_step: 1,
                max_steps: 6,
            },
            tools: Vec::new(),
            constraints: Vec::new(),
            max_tokens: Some(256),
        };

        let payload = build_payload(
            "claude-sonnet-4-6",
            &request,
            false,
            &ClaudePromptCaching { enabled: false },
            &ClaudeContextManagement {
                tool_result_clearing_enabled: false,
                tool_result_trigger_tokens: 90_000,
                server_side_compaction_enabled: false,
                compaction_trigger_tokens: 120_000,
            },
        );

        let content = must_some(
            payload
                .get("messages")
                .and_then(serde_json::Value::as_array)
                .and_then(|messages| messages.first())
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_array)
                .and_then(|content| content.first())
                .and_then(|block| block.get("text"))
                .and_then(serde_json::Value::as_str),
            "payload text block",
        );
        assert!(content.contains("Observed state snapshot"));
        assert!(content.contains("output_written"));
        assert!(!content.contains("- required inputs:"));
        assert!(!content.contains("named source hints"));
    }

    #[test]
    fn dynamic_context_block_includes_current_local_date_time() {
        let request = ReasonRequest {
            context: AssembledContext {
                identity: "retina".to_string(),
                task: "search the web for what is happening this weekend".to_string(),
                task_state: TaskState::default(),
                recent_context: None,
                tools: Vec::new(),
                memory_slice: Vec::new(),
                last_result: None,
                operator_guidance: None,
                current_step: 1,
                max_steps: 4,
            },
            tools: Vec::new(),
            constraints: Vec::new(),
            max_tokens: Some(256),
        };

        let block = crate::payload::build_dynamic_context_block(&request);
        assert!(block.contains("Current local date/time:"));
    }

    #[test]
    fn write_and_append_actions_have_no_approval_field() {
        let write = must(
            ClaudeAction {
                action_type: "write_file".to_string(),
                command: None,
                path: Some("note.txt".to_string()),
                root: None,
                pattern: None,
                query: None,
                content: Some("hello".to_string()),
                old_string: None,
                new_string: None,
                replace_all: None,
                include_content: None,
                cell_id: None,
                new_source: None,
                cell_type: None,
                edit_mode: None,
                recursive: None,
                max_entries: None,
                max_results: None,
                max_bytes: None,
                max_rows: None,
                max_chars: None,
                page_start: None,
                page_end: None,
                overwrite: Some(true),
                prompt: None,
                allowed_tools: None,
                denied_tools: None,
                require_approval: None,
                expect_change: None,
                note: None,
                message: None,
                task_complete: Some(false),
                intent_kind: None,
                deliverable: None,
                completion_basis: None,
                reasoning: None,
                server: None,
                tool: None,
                uri: None,
                input_json: None,
            }
            .into_reason_response(),
        );
        let append = must(
            ClaudeAction {
                action_type: "append_file".to_string(),
                command: None,
                path: Some("note.txt".to_string()),
                root: None,
                pattern: None,
                query: None,
                content: Some("more".to_string()),
                old_string: None,
                new_string: None,
                replace_all: None,
                include_content: None,
                cell_id: None,
                new_source: None,
                cell_type: None,
                edit_mode: None,
                recursive: None,
                max_entries: None,
                max_results: None,
                max_bytes: None,
                max_rows: None,
                max_chars: None,
                page_start: None,
                page_end: None,
                overwrite: None,
                prompt: None,
                allowed_tools: None,
                denied_tools: None,
                require_approval: None,
                expect_change: None,
                note: None,
                message: None,
                task_complete: Some(false),
                intent_kind: None,
                deliverable: None,
                completion_basis: None,
                reasoning: None,
                server: None,
                tool: None,
                uri: None,
                input_json: None,
            }
            .into_reason_response(),
        );

        match write.action {
            Action::WriteFile { .. } => {}
            other => panic!("expected write action, got {other:?}"),
        }
        match append.action {
            Action::AppendFile { .. } => {}
            other => panic!("expected append action, got {other:?}"),
        }
    }

    #[test]
    fn run_command_with_target_path_tracks_that_artifact_for_verification() {
        let response = must(
            ClaudeAction {
                action_type: "run_command".to_string(),
                command: Some("python script.py > out.txt".to_string()),
                path: Some("out.txt".to_string()),
                root: None,
                pattern: None,
                query: None,
                content: None,
                old_string: None,
                new_string: None,
                replace_all: None,
                include_content: None,
                cell_id: None,
                new_source: None,
                cell_type: None,
                edit_mode: None,
                recursive: None,
                max_entries: None,
                max_results: None,
                max_bytes: None,
                max_rows: None,
                max_chars: None,
                page_start: None,
                page_end: None,
                overwrite: None,
                prompt: None,
                allowed_tools: None,
                denied_tools: None,
                require_approval: None,
                expect_change: None,
                note: None,
                message: None,
                task_complete: Some(false),
                intent_kind: None,
                deliverable: None,
                completion_basis: None,
                reasoning: None,
                server: None,
                tool: None,
                uri: None,
                input_json: None,
            }
            .into_reason_response(),
        );

        match response.action {
            Action::RunCommand {
                expect_change,
                state_scope,
                ..
            } => {
                assert!(expect_change);
                assert_eq!(state_scope.tracked_paths.len(), 1);
                assert_eq!(
                    state_scope.tracked_paths[0].path,
                    std::path::PathBuf::from("out.txt")
                );
                assert!(state_scope.tracked_paths[0].include_content);
            }
            other => panic!("expected run_command action, got {other:?}"),
        }
    }

    #[test]
    fn claude_action_can_map_reasoner_framing_fields() {
        let response = must(
            ClaudeAction {
                action_type: "respond".to_string(),
                command: None,
                path: None,
                root: None,
                pattern: None,
                query: None,
                content: None,
                old_string: None,
                new_string: None,
                replace_all: None,
                include_content: None,
                cell_id: None,
                new_source: None,
                cell_type: None,
                edit_mode: None,
                recursive: None,
                max_entries: None,
                max_results: None,
                max_bytes: None,
                max_rows: None,
                max_chars: None,
                page_start: None,
                page_end: None,
                overwrite: None,
                prompt: None,
                allowed_tools: None,
                denied_tools: None,
                require_approval: None,
                expect_change: None,
                note: None,
                message: Some("done".to_string()),
                task_complete: Some(true),
                intent_kind: Some("answer".to_string()),
                deliverable: Some("summary of startup.md".to_string()),
                completion_basis: Some("read startup.md and extracted relevant lines".to_string()),
                reasoning: Some("responding from gathered evidence".to_string()),
                server: None,
                tool: None,
                uri: None,
                input_json: None,
            }
            .into_reason_response(),
        );

        let framing = response.framing.expect("expected framing");
        assert_eq!(framing.intent_kind, Some(TaskKind::Answer));
        assert_eq!(
            framing.deliverable.as_deref(),
            Some("summary of startup.md")
        );
        assert_eq!(
            framing.completion_basis.as_deref(),
            Some("read startup.md and extracted relevant lines")
        );
    }

    #[test]
    fn malformed_edit_file_action_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "edit_file".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: Some("before".to_string()),
            new_string: Some("after".to_string()),
            replace_all: Some(false),
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("edit_file"));
        assert!(message.contains("path"));
    }

    #[test]
    fn unknown_action_type_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "summarize_pdf".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(true),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("invalid Claude action type"));
        assert!(message.contains("summarize_pdf"));
    }

    #[test]
    fn respond_without_message_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "respond".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(true),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("respond"));
        assert!(message.contains("message"));
    }

    #[test]
    fn respond_accepts_content_as_message_alias() {
        let response = ClaudeAction {
            action_type: "respond".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: Some("hello from content".to_string()),
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(true),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_or_else(|error| panic!("respond alias should parse: {error}"));

        match response.action {
            Action::Respond { message, .. } => assert_eq!(message, "hello from content"),
            other => panic!("expected respond action, got {other:?}"),
        }
    }

    #[test]
    fn run_command_without_command_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "run_command".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("run_command"));
        assert!(message.contains("command"));
    }

    #[test]
    fn inspect_path_without_path_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "inspect_path".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("inspect_path"));
        assert!(message.contains("path"));
    }

    #[test]
    fn spawn_agent_without_prompt_returns_reasoning_error() {
        let error = ClaudeAction {
            action_type: "spawn_agent".to_string(),
            command: None,
            path: None,
            root: None,
            pattern: None,
            query: None,
            content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            server: None,
            tool: None,
            uri: None,
            input_json: None,
            cell_id: None,
            new_source: None,
            cell_type: None,
            edit_mode: None,
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_rows: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            prompt: None,
            allowed_tools: None,
            denied_tools: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            intent_kind: None,
            deliverable: None,
            completion_basis: None,
            reasoning: None,
        }
        .into_reason_response()
        .unwrap_err();

        let KernelError::Reasoning(message) = error else {
            panic!("expected reasoning error");
        };
        assert!(message.contains("spawn_agent"));
        assert!(message.contains("prompt"));
    }

    #[test]
    fn omitted_task_complete_defaults_false() {
        let response = must(
            ClaudeAction {
                action_type: "respond".to_string(),
                command: None,
                path: None,
                root: None,
                pattern: None,
                query: None,
                content: None,
                old_string: None,
                new_string: None,
                replace_all: None,
                server: None,
                tool: None,
                uri: None,
                input_json: None,
                cell_id: None,
                new_source: None,
                cell_type: None,
                edit_mode: None,
                include_content: None,
                recursive: None,
                max_entries: None,
                max_results: None,
                max_bytes: None,
                max_rows: None,
                max_chars: None,
                page_start: None,
                page_end: None,
                overwrite: None,
                prompt: None,
                allowed_tools: None,
                denied_tools: None,
                require_approval: None,
                expect_change: None,
                note: None,
                message: Some("done".to_string()),
                task_complete: None,
                intent_kind: None,
                deliverable: None,
                completion_basis: None,
                reasoning: None,
            }
            .into_reason_response(),
        );

        assert!(!response.task_complete);
    }
}
