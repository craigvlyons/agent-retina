use serde_json::json;
use std::env;

#[derive(Clone, Debug)]
pub(crate) struct ClaudePromptCaching {
    pub(crate) enabled: bool,
}

impl ClaudePromptCaching {
    pub(crate) fn from_env() -> Self {
        let enabled = env::var("RETINA_CLAUDE_PROMPT_CACHE")
            .map(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
            })
            .unwrap_or(true);
        Self { enabled }
    }

    pub(crate) fn cache_control_json(&self) -> serde_json::Value {
        json!({ "type": "ephemeral" })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClaudeContextManagement {
    pub(crate) tool_result_clearing_enabled: bool,
    pub(crate) tool_result_trigger_tokens: u32,
    pub(crate) server_side_compaction_enabled: bool,
    pub(crate) compaction_trigger_tokens: u32,
}

impl ClaudeContextManagement {
    pub(crate) fn from_env() -> Self {
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

pub(crate) fn anthropic_beta_header_value(
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

pub(crate) fn model_supports_server_compaction(model_id: &str) -> bool {
    matches!(model_id, "claude-sonnet-4-6" | "claude-opus-4-6")
}

fn env_flag(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(default)
}
