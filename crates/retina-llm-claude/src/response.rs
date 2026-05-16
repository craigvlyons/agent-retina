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
    pub(crate) old_string: Option<String>,
    pub(crate) new_string: Option<String>,
    pub(crate) replace_all: Option<bool>,
    pub(crate) server: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) uri: Option<String>,
    pub(crate) input_json: Option<serde_json::Value>,
    pub(crate) cell_id: Option<String>,
    pub(crate) new_source: Option<String>,
    pub(crate) cell_type: Option<String>,
    pub(crate) edit_mode: Option<String>,
    pub(crate) include_content: Option<bool>,
    pub(crate) recursive: Option<bool>,
    pub(crate) max_entries: Option<usize>,
    pub(crate) max_results: Option<usize>,
    pub(crate) max_bytes: Option<usize>,
    pub(crate) max_rows: Option<usize>,
    pub(crate) max_chars: Option<usize>,
    pub(crate) page_start: Option<usize>,
    pub(crate) page_end: Option<usize>,
    pub(crate) overwrite: Option<bool>,
    pub(crate) prompt: Option<String>,
    pub(crate) allowed_tools: Option<Vec<String>>,
    pub(crate) denied_tools: Option<Vec<String>>,
    pub(crate) require_approval: Option<bool>,
    pub(crate) expect_change: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) task_complete: Option<bool>,
    pub(crate) intent_kind: Option<String>,
    pub(crate) deliverable: Option<String>,
    pub(crate) completion_basis: Option<String>,
    pub(crate) reasoning: Option<String>,
}

impl ClaudeAction {
    pub(crate) fn into_reason_response(self) -> Result<ReasonResponse> {
        let task_complete = self.task_complete.unwrap_or(true);
        let framing = if self.intent_kind.is_some()
            || self.deliverable.is_some()
            || self.completion_basis.is_some()
        {
            Some(ReasonerTaskFraming {
                intent_kind: self.intent_kind.as_deref().and_then(parse_task_kind_hint),
                deliverable: self.deliverable,
                completion_basis: self.completion_basis,
            })
        } else {
            None
        };

        let action = match self.action_type.as_str() {
            "run_command" => Action::RunCommand {
                id: ActionId::new(),
                command: self.command.unwrap_or_else(|| "pwd".to_string()),
                cwd: None,
                require_approval: self.require_approval.unwrap_or(false),
                expect_change: self.expect_change.unwrap_or(self.path.is_some()),
                state_scope: HashScope {
                    tracked_paths: self
                        .path
                        .clone()
                        .map(|path| {
                            vec![TrackedPath {
                                path: path.into(),
                                include_content: true,
                            }]
                        })
                        .unwrap_or_default(),
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
            "ingest_structured_data" => Action::IngestStructuredData {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                max_rows: self.max_rows,
            },
            "extract_document_text" => Action::ExtractDocumentText {
                id: ActionId::new(),
                path: self.path.unwrap_or_else(|| ".".to_string()).into(),
                max_chars: self.max_chars,
                page_start: self.page_start,
                page_end: self.page_end,
            },
            "list_mcp_resources" => Action::ListMcpResources {
                id: ActionId::new(),
                server: self.server,
            },
            "read_mcp_resource" => Action::ReadMcpResource {
                id: ActionId::new(),
                server: required_string(self.server, "read_mcp_resource", "server")?,
                uri: required_string(self.uri, "read_mcp_resource", "uri")?,
            },
            "mcp_call" => Action::CallMcpTool {
                id: ActionId::new(),
                server: required_string(self.server, "mcp_call", "server")?,
                tool: required_string(self.tool, "mcp_call", "tool")?,
                input_json: self.input_json.ok_or_else(|| {
                    KernelError::Reasoning(
                        "invalid Claude action 'mcp_call': missing required field 'input_json'"
                            .to_string(),
                    )
                })?,
            },
            "write_file" => Action::WriteFile {
                id: ActionId::new(),
                path: required_string(self.path, "write_file", "path")?.into(),
                content: self.content.unwrap_or_default(),
                overwrite: self.overwrite.unwrap_or(false),
            },
            "edit_file" => Action::EditFile {
                id: ActionId::new(),
                path: required_string(self.path, "edit_file", "path")?.into(),
                old_string: required_string(self.old_string, "edit_file", "old_string")?,
                new_string: required_string(self.new_string, "edit_file", "new_string")?,
                replace_all: self.replace_all.unwrap_or(false),
            },
            "append_file" => Action::AppendFile {
                id: ActionId::new(),
                path: required_string(self.path, "append_file", "path")?.into(),
                content: self.content.unwrap_or_default(),
            },
            "edit_notebook" => Action::EditNotebook {
                id: ActionId::new(),
                path: required_string(self.path, "edit_notebook", "path")?.into(),
                cell_id: self.cell_id,
                new_source: notebook_source_for_mode(
                    &self.action_type,
                    self.edit_mode.as_deref(),
                    self.new_source,
                )?,
                cell_type: match self.cell_type.as_deref() {
                    Some("code") => Some(NotebookCellType::Code),
                    Some("markdown") => Some(NotebookCellType::Markdown),
                    Some(other) => {
                        return Err(KernelError::Reasoning(format!(
                            "invalid Claude action 'edit_notebook': unsupported cell_type '{}'; expected 'code' or 'markdown'",
                            other
                        )));
                    }
                    None => None,
                },
                edit_mode: match self.edit_mode.as_deref() {
                    Some("insert") => NotebookEditMode::Insert,
                    Some("delete") => NotebookEditMode::Delete,
                    _ => NotebookEditMode::Replace,
                },
            },
            "spawn_agent" => Action::SpawnAgent {
                id: ActionId::new(),
                prompt: self
                    .prompt
                    .or(self.message.clone())
                    .unwrap_or_else(|| "Investigate the delegated subtask.".to_string()),
                allowed_tools: self.allowed_tools.unwrap_or_default(),
                denied_tools: self.denied_tools.unwrap_or_default(),
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

        Ok(ReasonResponse {
            action,
            task_complete,
            framing,
            reasoning: self.reasoning,
            tokens_used: TokenUsage::default(),
        })
    }
}

fn required_string(value: Option<String>, action_type: &str, field: &str) -> Result<String> {
    value.filter(|text| !text.trim().is_empty()).ok_or_else(|| {
        KernelError::Reasoning(format!(
            "invalid Claude action '{}': missing required field '{}'",
            action_type, field
        ))
    })
}

fn notebook_source_for_mode(
    action_type: &str,
    edit_mode: Option<&str>,
    new_source: Option<String>,
) -> Result<String> {
    match edit_mode {
        Some("delete") => Ok(new_source.unwrap_or_default()),
        _ => required_string(new_source, action_type, "new_source"),
    }
}

fn parse_task_kind_hint(value: &str) -> Option<TaskKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "answer" => Some(TaskKind::Answer),
        "output" => Some(TaskKind::Output),
        "unknown" => Some(TaskKind::Unknown),
        _ => None,
    }
}
