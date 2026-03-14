use retina_types::*;
use serde::{Deserialize, Serialize};

pub(crate) fn map_claude_error(status: u16, body: &str, model_id: &str) -> KernelError {
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

pub(crate) fn extract_json_blob(text: &str) -> Result<String> {
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
pub(crate) struct ClaudeResponse {
    pub(crate) content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    pub(crate) usage: ClaudeUsage,
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
pub(crate) struct ClaudeContentBlock {
    #[serde(rename = "type")]
    pub(crate) block_type: String,
    pub(crate) text: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ClaudeUsage {
    #[serde(default)]
    pub(crate) input_tokens: u32,
    #[serde(default)]
    pub(crate) output_tokens: u32,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: u32,
    #[serde(default)]
    pub(crate) cache_read_input_tokens: u32,
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

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClaudeAction {
    #[serde(rename = "type")]
    pub(crate) action_type: String,
    pub(crate) command: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) root: Option<String>,
    pub(crate) pattern: Option<String>,
    pub(crate) query: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) include_content: Option<bool>,
    pub(crate) recursive: Option<bool>,
    pub(crate) max_entries: Option<usize>,
    pub(crate) max_results: Option<usize>,
    pub(crate) max_bytes: Option<usize>,
    pub(crate) max_chars: Option<usize>,
    pub(crate) page_start: Option<usize>,
    pub(crate) page_end: Option<usize>,
    pub(crate) overwrite: Option<bool>,
    pub(crate) require_approval: Option<bool>,
    pub(crate) expect_change: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) task_complete: Option<bool>,
    pub(crate) reasoning: Option<String>,
}

impl ClaudeAction {
    pub(crate) fn into_reason_response(self) -> ReasonResponse {
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
                page_start: self.page_start,
                page_end: self.page_end,
            },
            "write_file" => Action::WriteFile {
                id: ActionId::new(),
                path: self
                    .path
                    .unwrap_or_else(|| "retina-output.txt".to_string())
                    .into(),
                content: self.content.unwrap_or_default(),
                overwrite: self.overwrite.unwrap_or(false),
            },
            "append_file" => Action::AppendFile {
                id: ActionId::new(),
                path: self
                    .path
                    .unwrap_or_else(|| "retina-output.txt".to_string())
                    .into(),
                content: self.content.unwrap_or_default(),
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
