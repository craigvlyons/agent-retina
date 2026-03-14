use crate::ActionId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
        max_results: usize,
    },
    SearchText {
        id: ActionId,
        root: PathBuf,
        query: String,
        max_results: usize,
    },
    ReadFile {
        id: ActionId,
        path: PathBuf,
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
    WriteFile {
        id: ActionId,
        path: PathBuf,
        content: String,
        overwrite: bool,
    },
    AppendFile {
        id: ActionId,
        path: PathBuf,
        content: String,
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
            | Self::WriteFile { id, .. }
            | Self::AppendFile { id, .. }
            | Self::RecordNote { id, .. }
            | Self::Respond { id, .. } => id.clone(),
        }
    }

    pub fn expects_change(&self) -> bool {
        match self {
            Self::RunCommand { expect_change, .. } => *expect_change,
            Self::WriteFile { .. } | Self::AppendFile { .. } => true,
            Self::InspectPath { .. }
            | Self::InspectWorkingDirectory { .. }
            | Self::ListDirectory { .. }
            | Self::FindFiles { .. }
            | Self::SearchText { .. }
            | Self::ReadFile { .. }
            | Self::IngestStructuredData { .. }
            | Self::ExtractDocumentText { .. }
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
            | Self::AppendFile { path, .. } => HashScope {
                tracked_paths: vec![TrackedPath {
                    path: path.clone(),
                    include_content: true,
                }],
                include_working_directory: false,
                include_last_command: false,
            },
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
pub struct SearchMatch {
    pub path: PathBuf,
    pub line_number: usize,
    pub line: String,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredDataRow {
    pub row_number: usize,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionResult {
    Command(CommandResult),
    Inspection(WorldState),
    DirectoryListing {
        root: PathBuf,
        entries: Vec<DirectoryEntry>,
    },
    FileMatches {
        root: PathBuf,
        pattern: String,
        matches: Vec<PathBuf>,
    },
    FileRead {
        path: PathBuf,
        content: String,
        truncated: bool,
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
        matches: Vec<SearchMatch>,
    },
    FileWrite {
        path: PathBuf,
        bytes_written: usize,
        created: bool,
        overwritten: bool,
        appended: bool,
    },
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
