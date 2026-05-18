use crate::{ActionId, AgentId, TaskId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn normalize_name_for_mcp(name: &str) -> String {
    name.replace(
        |ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-',
        "_",
    )
}

pub fn build_mcp_tool_name(server: &str, tool: &str) -> String {
    format!(
        "mcp__{}__{}",
        normalize_name_for_mcp(server),
        normalize_name_for_mcp(tool)
    )
}

pub fn parse_mcp_tool_name(name: &str) -> Option<(String, String)> {
    let mut parts = name.split("__");
    let prefix = parts.next()?;
    let server = parts.next()?;
    if prefix != "mcp" || server.is_empty() {
        return None;
    }
    let tool = parts.collect::<Vec<_>>().join("__");
    if tool.is_empty() {
        return None;
    }
    Some((server.to_string(), tool))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    RunCommand {
        id: ActionId,
        command: String,
        cwd: Option<PathBuf>,
        require_approval: bool,
        expect_change: bool,
        state_scope: HashScope,
    },
    InspectPath {
        id: ActionId,
        path: PathBuf,
        include_content: bool,
    },
    InspectWorkingDirectory {
        id: ActionId,
    },
    ListDirectory {
        id: ActionId,
        path: PathBuf,
        recursive: bool,
        max_entries: usize,
    },
    FindFiles {
        id: ActionId,
        root: PathBuf,
        pattern: String,
        recursive: bool,
        max_results: usize,
        offset: usize,
    },
    SearchText {
        id: ActionId,
        root: PathBuf,
        query: String,
        max_results: usize,
        offset: usize,
        glob: Option<String>,
        case_insensitive: bool,
        output_mode: TextSearchOutputMode,
    },
    ReadFile {
        id: ActionId,
        path: PathBuf,
        start_line: Option<usize>,
        limit_lines: Option<usize>,
        max_bytes: Option<usize>,
    },
    IngestStructuredData {
        id: ActionId,
        path: PathBuf,
        max_rows: Option<usize>,
    },
    ExtractDocumentText {
        id: ActionId,
        path: PathBuf,
        max_chars: Option<usize>,
        page_start: Option<usize>,
        page_end: Option<usize>,
    },
    ListMcpResources {
        id: ActionId,
        server: Option<String>,
    },
    ReadMcpResource {
        id: ActionId,
        server: String,
        uri: String,
    },
    CallMcpTool {
        id: ActionId,
        server: String,
        tool: String,
        input_json: serde_json::Value,
        resolved_tool_name: Option<String>,
    },
    WriteFile {
        id: ActionId,
        path: PathBuf,
        content: String,
        overwrite: bool,
    },
    EditFile {
        id: ActionId,
        path: PathBuf,
        old_string: String,
        new_string: String,
        replace_all: bool,
    },
    AppendFile {
        id: ActionId,
        path: PathBuf,
        content: String,
    },
    EditNotebook {
        id: ActionId,
        path: PathBuf,
        cell_id: Option<String>,
        new_source: String,
        cell_type: Option<NotebookCellType>,
        edit_mode: NotebookEditMode,
    },
    SpawnAgent {
        id: ActionId,
        prompt: String,
        allowed_tools: Vec<String>,
        denied_tools: Vec<String>,
    },
    RecordNote {
        id: ActionId,
        note: String,
    },
    Respond {
        id: ActionId,
        message: String,
    },
}

impl Action {
    pub fn id(&self) -> ActionId {
        match self {
            Self::RunCommand { id, .. }
            | Self::InspectPath { id, .. }
            | Self::InspectWorkingDirectory { id }
            | Self::ListDirectory { id, .. }
            | Self::FindFiles { id, .. }
            | Self::SearchText { id, .. }
            | Self::ReadFile { id, .. }
            | Self::IngestStructuredData { id, .. }
            | Self::ExtractDocumentText { id, .. }
            | Self::ListMcpResources { id, .. }
            | Self::ReadMcpResource { id, .. }
            | Self::CallMcpTool { id, .. }
            | Self::WriteFile { id, .. }
            | Self::EditFile { id, .. }
            | Self::AppendFile { id, .. }
            | Self::EditNotebook { id, .. }
            | Self::SpawnAgent { id, .. }
            | Self::RecordNote { id, .. }
            | Self::Respond { id, .. } => id.clone(),
        }
    }

    pub fn expects_change(&self) -> bool {
        match self {
            Self::RunCommand { expect_change, .. } => *expect_change,
            Self::WriteFile { .. }
            | Self::EditFile { .. }
            | Self::AppendFile { .. }
            | Self::EditNotebook { .. } => true,
            Self::InspectPath { .. }
            | Self::InspectWorkingDirectory { .. }
            | Self::ListDirectory { .. }
            | Self::FindFiles { .. }
            | Self::SearchText { .. }
            | Self::ReadFile { .. }
            | Self::IngestStructuredData { .. }
            | Self::ExtractDocumentText { .. }
            | Self::ListMcpResources { .. }
            | Self::ReadMcpResource { .. }
            | Self::CallMcpTool { .. }
            | Self::SpawnAgent { .. }
            | Self::RecordNote { .. }
            | Self::Respond { .. } => false,
        }
    }

    pub fn hash_scope(&self) -> HashScope {
        match self {
            Self::RunCommand { state_scope, .. } => state_scope.clone(),
            Self::InspectPath {
                path,
                include_content,
                ..
            } => HashScope {
                tracked_paths: vec![TrackedPath {
                    path: path.clone(),
                    include_content: *include_content,
                }],
                include_working_directory: false,
                include_last_command: false,
            },
            Self::InspectWorkingDirectory { .. } => HashScope {
                tracked_paths: Vec::new(),
                include_working_directory: true,
                include_last_command: false,
            },
            Self::ListDirectory { path, .. } => HashScope {
                tracked_paths: vec![TrackedPath {
                    path: path.clone(),
                    include_content: false,
                }],
                include_working_directory: false,
                include_last_command: false,
            },
            Self::FindFiles { root, .. } | Self::SearchText { root, .. } => HashScope {
                tracked_paths: vec![TrackedPath {
                    path: root.clone(),
                    include_content: false,
                }],
                include_working_directory: false,
                include_last_command: false,
            },
            Self::ReadFile { path, .. }
            | Self::IngestStructuredData { path, .. }
            | Self::ExtractDocumentText { path, .. }
            | Self::WriteFile { path, .. }
            | Self::EditFile { path, .. }
            | Self::AppendFile { path, .. }
            | Self::EditNotebook { path, .. } => HashScope {
                tracked_paths: vec![TrackedPath {
                    path: path.clone(),
                    include_content: true,
                }],
                include_working_directory: false,
                include_last_command: false,
            },
            Self::ListMcpResources { .. }
            | Self::ReadMcpResource { .. }
            | Self::CallMcpTool { .. } => HashScope::default(),
            Self::SpawnAgent { .. } => HashScope::default(),
            Self::RecordNote { .. } | Self::Respond { .. } => HashScope::default(),
        }
    }

    pub fn approval_required_by_policy(&self) -> bool {
        match self {
            Self::RunCommand { command, .. } => classify_privileged_command(command).is_some(),
            _ => false,
        }
    }

    pub fn mark_approval_granted(&mut self) {
        if let Self::RunCommand {
            require_approval, ..
        } = self
        {
            *require_approval = true;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivilegedCommandKind {
    Delete,
    Kill,
}

pub fn classify_privileged_command(command: &str) -> Option<PrivilegedCommandKind> {
    let lower = command.to_lowercase();
    let tokens = lower
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ';' | '|' | '&' | '(' | ')' | '{' | '}' | '<' | '>' | '\'' | '"'
                )
        })
        .filter(|token| !token.is_empty());

    for token in tokens {
        match token {
            "rm" | "rmdir" | "unlink" => return Some(PrivilegedCommandKind::Delete),
            "kill" | "pkill" | "killall" => return Some(PrivilegedCommandKind::Kill),
            _ => {}
        }
    }

    if lower.contains(" -delete") {
        return Some(PrivilegedCommandKind::Delete);
    }

    None
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HashScope {
    pub tracked_paths: Vec<TrackedPath>,
    pub include_working_directory: bool,
    pub include_last_command: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackedPath {
    pub path: PathBuf,
    pub include_content: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorldState {
    pub cwd: PathBuf,
    pub files: Vec<PathState>,
    pub last_command: Option<CommandResult>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub scope: HashScope,
    pub cwd: PathBuf,
    pub cwd_hash: String,
    pub files: Vec<PathState>,
    pub last_command: Option<CommandResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathState {
    pub path: PathBuf,
    pub exists: bool,
    pub size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectoryListingSummary {
    pub total_entries: usize,
    pub file_count: usize,
    pub dir_count: usize,
    pub hidden_count: usize,
    pub sample_names: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub line: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextSearchOutputMode {
    Content,
    FilesWithMatches,
    Count,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub command: String,
    pub cwd: PathBuf,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub duration_ms: u64,
    pub cancelled: bool,
    pub termination: Option<String>,
    pub observed_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DelegatedTaskStatus {
    Completed,
    Failed,
    Blocked,
    Killed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DelegatedTaskResult {
    pub agent_id: AgentId,
    pub task_id: TaskId,
    pub parent_task_id: Option<TaskId>,
    pub status: DelegatedTaskStatus,
    pub summary: String,
    pub transcript_excerpt: Option<String>,
    pub output_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredDataRow {
    pub row_number: usize,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpRegistrySnapshot {
    pub servers: Vec<McpServerSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerSnapshot {
    pub name: String,
    pub tools: Vec<McpToolSummary>,
    pub resources: Vec<McpResourceSummary>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolSummary {
    pub server: String,
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    pub read_only: bool,
    pub destructive: bool,
    pub open_world: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpResourceSummary {
    pub server: String,
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpResourceContentItem {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob_base64: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpResourceReadResult {
    pub server: String,
    pub uri: String,
    pub contents: Vec<McpResourceContentItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpSearchOutcomeKind {
    GenericPortal,
    SpecificListing,
    NewsRoundup,
    SingleEvent,
    NoLocalSignal,
    ValidationError,
    ToolError,
    NonSearchResult,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpSearchHitSummary {
    pub url: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolCallResult {
    pub server: String,
    pub tool: String,
    pub content_preview: String,
    pub structured_content: Option<serde_json::Value>,
    pub is_error: bool,
    pub search_outcome_kind: Option<McpSearchOutcomeKind>,
    #[serde(default)]
    pub evidence_identities: Vec<String>,
    #[serde(default)]
    pub search_hits: Vec<McpSearchHitSummary>,
    pub primary_locator: Option<String>,
    pub evidence_summary: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileMutationKind {
    Create,
    Overwrite,
    Append,
    ExactEdit,
    NotebookReplace,
    NotebookInsert,
    NotebookDelete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchSummary {
    pub matched_occurrences: usize,
    pub replaced_occurrences: usize,
    pub old_preview: String,
    pub new_preview: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileArtifactPayload {
    pub original_content: Option<String>,
    pub final_content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotebookEditMode {
    Replace,
    Insert,
    Delete,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NotebookCellType {
    Code,
    Markdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionResult {
    Command(CommandResult),
    Inspection(WorldState),
    DirectoryListing {
        root: PathBuf,
        entries: Vec<DirectoryEntry>,
        summary: DirectoryListingSummary,
    },
    FileMatches {
        root: PathBuf,
        pattern: String,
        matches: Vec<PathBuf>,
        truncated: bool,
        applied_offset: usize,
    },
    FileRead {
        path: PathBuf,
        content: String,
        truncated: bool,
        start_line: usize,
        line_count: usize,
        total_lines: usize,
        total_bytes: usize,
        read_bytes: usize,
    },
    StructuredData {
        path: PathBuf,
        format: String,
        headers: Vec<String>,
        rows: Vec<StructuredDataRow>,
        total_rows: usize,
        truncated: bool,
        extraction_method: String,
    },
    DocumentText {
        path: PathBuf,
        content: String,
        truncated: bool,
        format: String,
        extraction_method: String,
        page_range: Option<DocumentPageRange>,
        structured_rows_detected: bool,
    },
    TextSearch {
        root: PathBuf,
        query: String,
        output_mode: TextSearchOutputMode,
        matches: Vec<SearchMatch>,
        content: Option<String>,
        filenames: Vec<PathBuf>,
        num_files: usize,
        num_matches: usize,
        truncated: bool,
        applied_offset: usize,
        glob: Option<String>,
        case_insensitive: bool,
    },
    McpResources {
        server: Option<String>,
        resources: Vec<McpResourceSummary>,
    },
    McpResourceRead(McpResourceReadResult),
    McpToolCall(McpToolCallResult),
    FileWrite {
        path: PathBuf,
        mutation_kind: FileMutationKind,
        bytes_written: usize,
        created: bool,
        overwritten: bool,
        appended: bool,
        original_hash: Option<String>,
        updated_hash: String,
        changed_line_count: usize,
        patch_summary: Option<PatchSummary>,
        preview_excerpt: Option<String>,
        artifact: FileArtifactPayload,
    },
    DelegatedTask(DelegatedTaskResult),
    NoteRecorded {
        note: String,
    },
    Response {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateDelta {
    pub kind: StateDeltaKind,
    pub summary: String,
    pub changed_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocumentPageRange {
    pub start_page: usize,
    pub end_page: usize,
}

impl DocumentPageRange {
    pub fn render(&self) -> String {
        if self.start_page == self.end_page {
            format!("page {}", self.start_page)
        } else {
            format!("pages {}-{}", self.start_page, self.end_page)
        }
    }
}

impl StateDelta {
    pub fn unchanged() -> Self {
        Self {
            kind: StateDeltaKind::Unchanged,
            summary: "no state change detected".to_string(),
            changed_paths: Vec::new(),
        }
    }

    pub fn utility_score(&self) -> f64 {
        match self.kind {
            StateDeltaKind::ChangedAsExpected => 1.0,
            StateDeltaKind::ChangedUnexpectedly => -0.5,
            StateDeltaKind::Unchanged => -0.25,
            StateDeltaKind::Error => -1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum StateDeltaKind {
    ChangedAsExpected,
    Unchanged,
    ChangedUnexpectedly,
    Error,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Outcome {
    Success(ActionResult),
    Failure(String),
    Blocked(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub action: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ApprovalResponse {
    Approved,
    Denied,
    Cancelled,
}
