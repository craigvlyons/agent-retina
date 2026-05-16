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

pub(crate) fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429 | 500 | 502 | 503 | 504)
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

    if let Some(candidate) = extract_balanced_json_object(trimmed) {
        if serde_json::from_str::<serde_json::Value>(&candidate).is_ok() {
            return Ok(candidate);
        }
    }

    Err(KernelError::Reasoning(format!(
        "Claude did not return parseable JSON. Raw response: {}",
        trimmed
    )))
}

fn extract_balanced_json_object(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    let end = start + offset;
                    return Some(text[start..=end].trim().to_string());
                }
            }
            _ => {}
        }
    }

    None
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
    #[serde(default)]
    pub(crate) input: serde_json::Value,
    pub(crate) task_complete: Option<bool>,
    pub(crate) intent_kind: Option<String>,
    pub(crate) deliverable: Option<String>,
    pub(crate) completion_basis: Option<String>,
    pub(crate) reasoning: Option<String>,
}

impl ClaudeAction {
    pub(crate) fn into_reason_response(self) -> Result<ReasonResponse> {
        let task_complete = self.task_complete.unwrap_or(false);
        let intent_kind = self.intent_kind.clone();
        let deliverable = self.deliverable.clone();
        let completion_basis = self.completion_basis.clone();
        let reasoning = self.reasoning.clone();
        let framing =
            if intent_kind.is_some() || deliverable.is_some() || completion_basis.is_some() {
                Some(ReasonerTaskFraming {
                    intent_kind: intent_kind.as_deref().and_then(parse_task_kind_hint),
                    deliverable,
                    completion_basis,
                })
            } else {
                None
            };

        let action = match self.action_type.as_str() {
            "run_command" => Action::RunCommand {
                id: ActionId::new(),
                command: self.required_string("command")?,
                cwd: None,
                require_approval: self.optional_bool("require_approval")?.unwrap_or(false),
                expect_change: self
                    .optional_bool("expect_change")?
                    .unwrap_or(self.optional_string("path")?.is_some()),
                state_scope: HashScope {
                    tracked_paths: self
                        .optional_string("path")?
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
                path: self.required_string("path")?.into(),
                include_content: self.optional_bool("include_content")?.unwrap_or(true),
            },
            "list_directory" => Action::ListDirectory {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                recursive: self.optional_bool("recursive")?.unwrap_or(false),
                max_entries: self.optional_usize("max_entries")?.unwrap_or(100),
            },
            "find_files" => Action::FindFiles {
                id: ActionId::new(),
                root: self.required_string("root")?.into(),
                pattern: self.required_string("pattern")?,
                recursive: self.optional_bool("recursive")?.unwrap_or(true),
                max_results: self.optional_usize("max_results")?.unwrap_or(50),
            },
            "search_text" => Action::SearchText {
                id: ActionId::new(),
                root: self.required_string("root")?.into(),
                query: self.required_string("query")?,
                max_results: self.optional_usize("max_results")?.unwrap_or(25),
            },
            "read_file" => Action::ReadFile {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                max_bytes: self.optional_usize("max_bytes")?,
            },
            "ingest_structured_data" => Action::IngestStructuredData {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                max_rows: self.optional_usize("max_rows")?,
            },
            "extract_document_text" => Action::ExtractDocumentText {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                max_chars: self.optional_usize("max_chars")?,
                page_start: self.optional_usize("page_start")?,
                page_end: self.optional_usize("page_end")?,
            },
            "list_mcp_resources" => Action::ListMcpResources {
                id: ActionId::new(),
                server: self.optional_string("server")?,
            },
            "read_mcp_resource" => Action::ReadMcpResource {
                id: ActionId::new(),
                server: self.required_string("server")?,
                uri: self.required_string("uri")?,
            },
            "write_file" => Action::WriteFile {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                content: self.required_string("content")?,
                overwrite: self.optional_bool("overwrite")?.unwrap_or(false),
            },
            "edit_file" => Action::EditFile {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                old_string: self.required_string("old_string")?,
                new_string: self.required_string("new_string")?,
                replace_all: self.optional_bool("replace_all")?.unwrap_or(false),
            },
            "append_file" => Action::AppendFile {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                content: self.required_string("content")?,
            },
            "edit_notebook" => Action::EditNotebook {
                id: ActionId::new(),
                path: self.required_string("path")?.into(),
                cell_id: self.optional_string("cell_id")?,
                new_source: notebook_source_for_mode(
                    &self.action_type,
                    self.optional_string("edit_mode")?.as_deref(),
                    self.optional_string("new_source")?,
                )?,
                cell_type: match self.optional_string("cell_type")?.as_deref() {
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
                edit_mode: match self.optional_string("edit_mode")?.as_deref() {
                    Some("insert") => NotebookEditMode::Insert,
                    Some("delete") => NotebookEditMode::Delete,
                    _ => NotebookEditMode::Replace,
                },
            },
            "agent_spawn" => Action::SpawnAgent {
                id: ActionId::new(),
                prompt: self.required_string("prompt")?,
                allowed_tools: self
                    .optional_string_vec("allowed_tools")?
                    .unwrap_or_default(),
                denied_tools: self
                    .optional_string_vec("denied_tools")?
                    .unwrap_or_default(),
            },
            "record_note" => Action::RecordNote {
                id: ActionId::new(),
                note: self.required_string("note")?,
            },
            "respond" => Action::Respond {
                id: ActionId::new(),
                message: self.required_string("message")?,
            },
            other => {
                if let Some((server, tool)) = parse_mcp_tool_name(other) {
                    let _ = self.input_object()?;
                    Action::CallMcpTool {
                        id: ActionId::new(),
                        server,
                        tool,
                        input_json: self.input.clone(),
                        resolved_tool_name: Some(other.to_string()),
                    }
                } else {
                    return Err(KernelError::Reasoning(format!(
                        "invalid Claude action type '{}'",
                        other
                    )));
                }
            }
        };

        Ok(ReasonResponse {
            action,
            task_complete,
            framing,
            reasoning,
            tokens_used: TokenUsage::default(),
        })
    }

    fn input_object(&self) -> Result<&serde_json::Map<String, serde_json::Value>> {
        self.input.as_object().ok_or_else(|| {
            KernelError::Reasoning(format!(
                "invalid Claude action '{}': missing required object field 'input'",
                self.action_type
            ))
        })
    }

    fn optional_string(&self, field: &str) -> Result<Option<String>> {
        let input = self.input_object()?;
        match input.get(field) {
            Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
                Ok(Some(value.clone()))
            }
            Some(serde_json::Value::String(_)) => Ok(None),
            Some(_) => Err(self.invalid_input_field(field, "string")),
            None => Ok(None),
        }
    }

    fn required_string(&self, field: &str) -> Result<String> {
        self.optional_string(field)?.ok_or_else(|| {
            KernelError::Reasoning(format!(
                "invalid Claude action '{}': missing required input field '{}'",
                self.action_type, field
            ))
        })
    }

    fn optional_bool(&self, field: &str) -> Result<Option<bool>> {
        let input = self.input_object()?;
        match input.get(field) {
            Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(self.invalid_input_field(field, "boolean")),
            None => Ok(None),
        }
    }

    fn optional_usize(&self, field: &str) -> Result<Option<usize>> {
        let input = self.input_object()?;
        match input.get(field) {
            Some(serde_json::Value::Number(value)) => value
                .as_u64()
                .map(|n| n as usize)
                .map(Some)
                .ok_or_else(|| self.invalid_input_field(field, "integer")),
            Some(_) => Err(self.invalid_input_field(field, "integer")),
            None => Ok(None),
        }
    }

    fn optional_string_vec(&self, field: &str) -> Result<Option<Vec<String>>> {
        let input = self.input_object()?;
        match input.get(field) {
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    serde_json::Value::String(text) if !text.trim().is_empty() => Ok(text.clone()),
                    serde_json::Value::String(_) => {
                        Err(self.invalid_input_field(field, "non-empty string array"))
                    }
                    _ => Err(self.invalid_input_field(field, "string array")),
                })
                .collect::<Result<Vec<_>>>()
                .map(Some),
            Some(_) => Err(self.invalid_input_field(field, "string array")),
            None => Ok(None),
        }
    }

    fn invalid_input_field(&self, field: &str, expected: &str) -> KernelError {
        KernelError::Reasoning(format!(
            "invalid Claude action '{}': input field '{}' must be a {}",
            self.action_type, field, expected
        ))
    }
}

fn notebook_source_for_mode(
    action_type: &str,
    edit_mode: Option<&str>,
    new_source: Option<String>,
) -> Result<String> {
    match edit_mode {
        Some("delete") => Ok(new_source.unwrap_or_default()),
        _ => new_source
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                KernelError::Reasoning(format!(
                    "invalid Claude action '{}': missing required input field 'new_source'",
                    action_type
                ))
            }),
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
