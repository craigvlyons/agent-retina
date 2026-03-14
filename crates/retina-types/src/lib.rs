use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, KernelError>;
pub type EventPayload = Value;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(AgentId);
id_type!(TaskId);
id_type!(SessionId);
id_type!(IntentId);
id_type!(ActionId);
id_type!(EventId);
id_type!(ExperienceId);
id_type!(KnowledgeId);
id_type!(RuleId);
id_type!(ToolId);

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum KernelError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("shell execution error: {0}")]
    Execution(String),
    #[error("reasoning error: {0}")]
    Reasoning(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("approval denied: {0}")]
    ApprovalDenied(String),
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl From<std::io::Error> for KernelError {
    fn from(value: std::io::Error) -> Self {
        Self::Execution(value.to_string())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub metadata: BTreeMap<String, String>,
}

impl Task {
    pub fn new(agent_id: AgentId, description: impl Into<String>) -> Self {
        Self {
            id: TaskId::new(),
            session_id: SessionId::new(),
            agent_id,
            description: description.into(),
            created_at: Utc::now(),
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intent {
    pub id: IntentId,
    pub task_id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub objective: String,
    pub action: Option<Action>,
    pub expects_change: bool,
    pub hash_scope: HashScope,
    pub created_at: DateTime<Utc>,
    pub metadata: BTreeMap<String, String>,
}

impl Intent {
    pub fn from_task(task: &Task) -> Self {
        Self {
            id: IntentId::new(),
            task_id: task.id.clone(),
            session_id: task.session_id.clone(),
            agent_id: task.agent_id.clone(),
            objective: task.description.clone(),
            action: None,
            expects_change: true,
            hash_scope: HashScope::default(),
            created_at: Utc::now(),
            metadata: task.metadata.clone(),
        }
    }
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
    ExtractDocumentText {
        id: ActionId,
        path: PathBuf,
        max_chars: Option<usize>,
    },
    WriteFile {
        id: ActionId,
        path: PathBuf,
        content: String,
        overwrite: bool,
        require_approval: bool,
    },
    AppendFile {
        id: ActionId,
        path: PathBuf,
        content: String,
        require_approval: bool,
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
    DocumentText {
        path: PathBuf,
        content: String,
        truncated: bool,
        format: String,
    },
    TextSearch {
        root: PathBuf,
        query: String,
        matches: Vec<SearchMatch>,
    },
    FileWrite {
        path: PathBuf,
        bytes_written: usize,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Outcome {
    Success(ActionResult),
    Failure(String),
    Blocked(String),
}

#[derive(Clone, Debug)]
pub struct ExecutionConfig {
    pub max_steps: usize,
    pub control: Option<ExecutionControlHandle>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_steps: 4,
            control: None,
        }
    }
}

#[derive(Debug)]
struct ExecutionControlState {
    cancel_requested: AtomicBool,
    operator_guidance: Mutex<Option<String>>,
}

#[derive(Clone, Debug)]
pub struct ExecutionControl {
    state: Arc<ExecutionControlState>,
}

#[derive(Clone, Debug)]
pub struct ExecutionControlHandle {
    state: Arc<ExecutionControlState>,
}

impl ExecutionControl {
    pub fn new() -> Self {
        Self {
            state: Arc::new(ExecutionControlState {
                cancel_requested: AtomicBool::new(false),
                operator_guidance: Mutex::new(None),
            }),
        }
    }

    pub fn handle(&self) -> ExecutionControlHandle {
        ExecutionControlHandle {
            state: Arc::clone(&self.state),
        }
    }
}

impl Default for ExecutionControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionControlHandle {
    pub fn request_cancel(&self) {
        self.state.cancel_requested.store(true, Ordering::Relaxed);
    }

    pub fn is_cancel_requested(&self) -> bool {
        self.state.cancel_requested.load(Ordering::Relaxed)
    }

    pub fn queue_guidance(&self, guidance: impl Into<String>) {
        *recover_mutex(&self.state.operator_guidance) = Some(guidance.into());
    }

    pub fn take_guidance(&self) -> Option<String> {
        recover_mutex(&self.state.operator_guidance).take()
    }
}

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Experience {
    pub id: Option<ExperienceId>,
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub intent_id: IntentId,
    pub action_summary: String,
    pub outcome: String,
    pub utility: f64,
    pub created_at: DateTime<Utc>,
    pub metadata: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeNode {
    pub id: Option<KnowledgeId>,
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReflexiveRule {
    pub id: Option<RuleId>,
    pub name: String,
    pub condition: RuleCondition,
    pub action: RuleAction,
    pub confidence: f64,
    pub active: bool,
    pub last_fired: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RuleCondition {
    TaskContains(String),
    LastDelta(StateDeltaKind),
    CommandContains(String),
    Always,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RuleAction {
    Block(String),
    UseAction(Action),
    AddNote(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolRecord {
    pub id: Option<ToolId>,
    pub name: String,
    pub description: String,
    pub source_lang: SourceLanguage,
    pub test_status: String,
    pub metadata: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub event_id: EventId,
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub agent_id: AgentId,
    pub timestamp: DateTime<Utc>,
    pub event_type: TimelineEventType,
    pub intent_id: Option<IntentId>,
    pub action_id: Option<ActionId>,
    pub pre_state_hash: Option<String>,
    pub post_state_hash: Option<String>,
    pub delta_summary: Option<String>,
    pub duration_ms: Option<u64>,
    pub payload_json: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimelineEventType {
    TaskReceived,
    TaskContextAssembled,
    IntentCreated,
    ReflexChecked,
    CircuitBreakerChecked,
    TaskCancelRequested,
    OperatorGuidanceQueued,
    ApprovalPromptShown,
    ApprovalPromptResolved,
    PreStateCaptured,
    ReasonerCalled,
    ReflexSelected,
    ActionDispatched,
    ActionResultReceived,
    PostStateCaptured,
    StateDeltaComputed,
    ExperiencePersisted,
    UtilityScored,
    ConsolidationCompleted,
    TaskCompacted,
    ReflectionRequested,
    ReflectionCompleted,
    TaskStepCompleted,
    TaskCancelled,
    TaskCompleted,
    TaskFailed,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskState {
    pub goal: TaskGoal,
    pub progress: TaskProgress,
    pub frontier: TaskFrontier,
    pub recent_actions: Vec<RecentActionSummary>,
    pub working_sources: Vec<WorkingSource>,
    pub artifact_references: Vec<ArtifactReference>,
    pub avoid: Vec<AvoidRule>,
    pub compaction: Option<CompactionSnapshot>,
}

impl TaskState {
    pub fn with_constraints(mut self, constraints: Vec<String>) -> Self {
        self.goal.constraints = constraints;
        self
    }

    pub fn render(&self) -> String {
        let success_criteria = render_list(&self.goal.success_criteria);
        let constraints = render_list(&self.goal.constraints);
        let completed = render_list(&self.progress.completed_checkpoints);
        let verified = render_list(&self.progress.verified_facts);
        let open_questions = render_list(&self.frontier.open_questions);
        let blockers = render_list(&self.frontier.blockers);
        let next_action = self
            .frontier
            .next_action_hint
            .clone()
            .unwrap_or_else(|| "none".to_string());
        let recent_actions = if self.recent_actions.is_empty() {
            "none".to_string()
        } else {
            self.recent_actions
                .iter()
                .map(RecentActionSummary::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let working_sources = if self.working_sources.is_empty() {
            "none".to_string()
        } else {
            self.working_sources
                .iter()
                .map(WorkingSource::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let artifacts = if self.artifact_references.is_empty() {
            "none".to_string()
        } else {
            self.artifact_references
                .iter()
                .map(ArtifactReference::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let avoid = if self.avoid.is_empty() {
            "none".to_string()
        } else {
            self.avoid
                .iter()
                .map(AvoidRule::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let compaction = self
            .compaction
            .as_ref()
            .map(CompactionSnapshot::render)
            .unwrap_or_else(|| "none".to_string());

        format!(
            "Goal:\n- objective: {}\n- success_criteria:\n{}\n- constraints:\n{}\n\nProgress:\n- phase: {}\n- step: {} / {}\n- completed:\n{}\n- verified_facts:\n{}\n\nFrontier:\n- next_action_hint: {}\n- open_questions:\n{}\n- blockers:\n{}\n\nRecent meaningful actions:\n{}\n\nWorking sources:\n{}\n\nArtifact references:\n{}\n\nAvoid:\n{}\n\nCompaction:\n{}",
            self.goal.objective,
            success_criteria,
            constraints,
            self.progress.current_phase,
            self.progress.current_step,
            self.progress.max_steps,
            completed,
            verified,
            next_action,
            open_questions,
            blockers,
            recent_actions,
            working_sources,
            artifacts,
            avoid,
            compaction
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskGoal {
    pub objective: String,
    pub success_criteria: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskProgress {
    pub current_phase: String,
    pub current_step: usize,
    pub max_steps: usize,
    pub completed_checkpoints: Vec<String>,
    pub verified_facts: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskFrontier {
    pub next_action_hint: Option<String>,
    pub open_questions: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentActionSummary {
    pub step: usize,
    pub action: String,
    pub outcome: String,
    pub artifact_refs: Vec<ArtifactReference>,
}

impl RecentActionSummary {
    pub fn render(&self) -> String {
        let refs = if self.artifact_refs.is_empty() {
            String::new()
        } else {
            format!(
                " [{}]",
                self.artifact_refs
                    .iter()
                    .map(|item| item.locator.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "- step {}: {} -> {}{}",
            self.step, self.action, self.outcome, refs
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkingSource {
    pub kind: String,
    pub locator: String,
    pub role: String,
    pub status: String,
    pub why_it_matters: String,
    pub last_used_step: usize,
    pub evidence_refs: Vec<String>,
}

impl WorkingSource {
    pub fn render(&self) -> String {
        format!(
            "- {} [{}|{}] {}",
            self.locator, self.kind, self.role, self.status
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub kind: String,
    pub locator: String,
    pub status: String,
}

impl ArtifactReference {
    pub fn render(&self) -> String {
        format!("- {} [{}|{}]", self.locator, self.kind, self.status)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvoidRule {
    pub label: String,
    pub reason: String,
}

impl AvoidRule {
    pub fn render(&self) -> String {
        format!("- {}: {}", self.label, self.reason)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionSnapshot {
    pub reason: String,
    pub score_explanations: Vec<CompactionScoreExplanation>,
}

impl CompactionSnapshot {
    pub fn render(&self) -> String {
        let scores = if self.score_explanations.is_empty() {
            "  - none".to_string()
        } else {
            self.score_explanations
                .iter()
                .map(CompactionScoreExplanation::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!("- reason: {}\n- ranking:\n{}", self.reason, scores)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionScoreExplanation {
    pub item_kind: String,
    pub locator: String,
    pub decision: String,
    pub rationale: String,
}

impl CompactionScoreExplanation {
    pub fn render(&self) -> String {
        format!(
            "  - {} {} => {} ({})",
            self.item_kind, self.locator, self.decision, self.rationale
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembledContext {
    pub identity: String,
    pub task: String,
    pub task_state: TaskState,
    pub tools: Vec<ToolDescriptor>,
    pub memory_slice: Vec<String>,
    pub last_result: Option<String>,
    pub last_result_summary: Option<String>,
    pub recent_steps: Vec<String>,
    pub operator_guidance: Option<String>,
    pub current_step: usize,
    pub max_steps: usize,
}

impl AssembledContext {
    pub fn render(&self) -> String {
        let tools = self
            .tools
            .iter()
            .map(|tool| format!("- {}: {}", tool.name, tool.description))
            .collect::<Vec<_>>()
            .join("\n");
        let memory = if self.memory_slice.is_empty() {
            "none".to_string()
        } else {
            self.memory_slice.join("\n")
        };
        let recent_steps = if self.recent_steps.is_empty() {
            "none".to_string()
        } else {
            self.recent_steps.join("\n")
        };
        let operator_guidance = self
            .operator_guidance
            .clone()
            .unwrap_or_else(|| "none".to_string());
        format!(
            "Identity:\n{}\n\nTask:\n{}\n\nTask state:\n{}\n\nTools:\n{}\n\nMemory:\n{}\n\nRecent steps:\n{}\n\nOperator guidance:\n{}\n\nLast result summary:\n{}\n\nLast result:\n{}",
            self.identity,
            self.task,
            self.task_state.render(),
            tools,
            memory,
            recent_steps,
            operator_guidance,
            self.last_result_summary
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            self.last_result
                .clone()
                .unwrap_or_else(|| "none".to_string())
        )
    }
}

fn render_list(items: &[String]) -> String {
    if items.is_empty() {
        "  - none".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("  - {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonRequest {
    pub context: AssembledContext,
    pub tools: Vec<ToolDescriptor>,
    pub constraints: Vec<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonResponse {
    pub action: Action,
    pub task_complete: bool,
    pub reasoning: Option<String>,
    pub tokens_used: TokenUsage,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonerCapabilities {
    pub max_context_tokens: u32,
    pub supports_tool_use: bool,
    pub supports_vision: bool,
    pub supports_caching: bool,
    pub model_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellCapabilities {
    pub can_execute_commands: bool,
    pub can_read_files: bool,
    pub can_write_files: bool,
    pub can_search_files: bool,
    pub can_extract_documents: bool,
    pub can_write_notes: bool,
    pub can_respond_text: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HardConstraint {
    NoNetworkShellActions,
    DestructiveOperationsRequireApproval,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RoutingDecision {
    HandleDirectly,
    RouteToExisting(AgentId),
    Reactivate(AgentId),
    SpawnSpecialist { domain: String, capability: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingCandidate {
    pub agent_id: AgentId,
    pub domain: String,
    pub status: AgentStatus,
    pub capability_match: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingAssessment {
    pub effective_decision: RoutingDecision,
    pub recommended_decision: RoutingDecision,
    pub candidates: Vec<RoutingCandidate>,
    pub rationale: String,
    pub network_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentManifest {
    pub agent_id: AgentId,
    pub domain: String,
    pub status: AgentStatus,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub parent_agent_id: Option<AgentId>,
    pub capabilities: Vec<String>,
    pub authority: AgentAuthority,
    pub lifecycle: AgentLifecycle,
    pub budget: AgentBudget,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentStatus {
    Spawned,
    Running,
    Idle,
    Archived,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentLifecyclePhase {
    Bootstrapping,
    Ready,
    Busy,
    CoolingDown,
    Archived,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentLifecycle {
    pub phase: AgentLifecyclePhase,
    pub last_active_at: Option<DateTime<Utc>>,
    pub last_task_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub status_reason: Option<String>,
}

impl AgentLifecycle {
    pub fn ready() -> Self {
        Self {
            phase: AgentLifecyclePhase::Ready,
            last_active_at: None,
            last_task_at: None,
            archived_at: None,
            status_reason: None,
        }
    }

    pub fn transition(
        &mut self,
        phase: AgentLifecyclePhase,
        timestamp: DateTime<Utc>,
        reason: Option<String>,
    ) {
        self.phase = phase.clone();
        self.last_active_at = Some(timestamp);
        if matches!(
            phase,
            AgentLifecyclePhase::Busy | AgentLifecyclePhase::CoolingDown
        ) {
            self.last_task_at = Some(timestamp);
        }
        if matches!(phase, AgentLifecyclePhase::Archived) {
            self.archived_at = Some(timestamp);
        }
        if let Some(reason) = reason {
            self.status_reason = Some(reason);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentBudget {
    pub max_steps_per_task: usize,
    pub max_reasoner_calls_per_task: usize,
    pub max_tokens_per_task: u32,
    pub idle_archive_after_hours: Option<u64>,
}

impl Default for AgentBudget {
    fn default() -> Self {
        Self {
            max_steps_per_task: 8,
            max_reasoner_calls_per_task: 8,
            max_tokens_per_task: 8_192,
            idle_archive_after_hours: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentAuthority {
    pub allow_command_execution: bool,
    pub allow_file_reads: bool,
    pub allow_file_writes: bool,
    pub allow_file_search: bool,
    pub allow_notes: bool,
    pub allow_text_responses: bool,
    pub accessible_roots: Vec<PathBuf>,
}

impl Default for AgentAuthority {
    fn default() -> Self {
        Self {
            allow_command_execution: true,
            allow_file_reads: true,
            allow_file_writes: true,
            allow_file_search: true,
            allow_notes: true,
            allow_text_responses: true,
            accessible_roots: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeUpdate {
    pub confidence: Option<f64>,
    pub metadata: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleUpdate {
    pub confidence: Option<f64>,
    pub active: Option<bool>,
    pub last_fired: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    pub max_recent_states: usize,
    pub min_successful_repeats: usize,
    pub min_success_utility: f64,
    pub min_rule_confidence: f64,
    pub stale_knowledge_days: Option<u64>,
    pub optimize_after_cleanup: bool,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            max_recent_states: 0,
            min_successful_repeats: 3,
            min_success_utility: 0.5,
            min_rule_confidence: 0.8,
            stale_knowledge_days: None,
            optimize_after_cleanup: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub merged_knowledge: usize,
    pub promoted_rules: usize,
    pub compacted_events: usize,
    pub decayed_knowledge: usize,
    pub optimized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceLanguage {
    Rust,
    Other(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSource {
    pub language: SourceLanguage,
    pub code: String,
    pub dependencies: Vec<Dependency>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledTool {
    pub binary: Vec<u8>,
    pub source_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolTest {
    pub name: String,
    pub input: Value,
    pub expected: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TestReport {
    pub passed: bool,
    pub executed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FabricatorCapabilities {
    pub allows_filesystem: bool,
    pub allows_network: bool,
    pub memory_limit_bytes: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from: AgentId,
    pub to: AgentId,
    pub kind: MessageKind,
    pub payload: Value,
    pub correlation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessageKind {
    TaskRequest,
    TaskResult,
    DataHandoff,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCard {
    pub agent_id: AgentId,
    pub domain: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub status: AgentStatus,
    pub lifecycle_phase: AgentLifecyclePhase,
    pub last_active_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRegistrySnapshot {
    pub updated_at: DateTime<Utc>,
    pub active_agents: Vec<AgentCard>,
    pub archived_agents: Vec<AgentCard>,
}

impl Default for AgentRegistrySnapshot {
    fn default() -> Self {
        Self {
            updated_at: Utc::now(),
            active_agents: Vec::new(),
            archived_agents: Vec::new(),
        }
    }
}
