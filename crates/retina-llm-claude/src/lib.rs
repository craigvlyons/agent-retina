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
                        shape: TaskShape::default(),
                        progress: TaskProgress {
                            current_phase: "starting".to_string(),
                            current_step: 1,
                            max_steps: 4,
                            completed_checkpoints: Vec::new(),
                            verified_facts: Vec::new(),
                            required_inputs: 0,
                            satisfied_inputs: 0,
                            output_written: false,
                            output_verified: false,
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

    #[test]
    fn stable_instructions_prefer_best_verifiable_step_over_timid_discovery_bias() {
        let instructions = build_stable_instructions(false);
        assert!(instructions.contains("best next verifiable step"));
        assert!(instructions.contains("target those artifacts directly"));
        assert!(!instructions.contains("smallest useful next step"));
        assert!(!instructions.contains("Prefer structured filesystem actions over shell commands"));
        assert!(!instructions.contains("Only use run_command for an explicit shell command"));
    }

    #[test]
    fn write_and_append_actions_have_no_approval_field() {
        let write = ClaudeAction {
            action_type: "write_file".to_string(),
            command: None,
            path: Some("note.txt".to_string()),
            root: None,
            pattern: None,
            query: None,
            content: Some("hello".to_string()),
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: Some(true),
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            reasoning: None,
        }
        .into_reason_response();
        let append = ClaudeAction {
            action_type: "append_file".to_string(),
            command: None,
            path: Some("note.txt".to_string()),
            root: None,
            pattern: None,
            query: None,
            content: Some("more".to_string()),
            include_content: None,
            recursive: None,
            max_entries: None,
            max_results: None,
            max_bytes: None,
            max_chars: None,
            page_start: None,
            page_end: None,
            overwrite: None,
            require_approval: None,
            expect_change: None,
            note: None,
            message: None,
            task_complete: Some(false),
            reasoning: None,
        }
        .into_reason_response();

        match write.action {
            Action::WriteFile { .. } => {}
            other => panic!("expected write action, got {other:?}"),
        }
        match append.action {
            Action::AppendFile { .. } => {}
            other => panic!("expected append action, got {other:?}"),
        }
    }
}
