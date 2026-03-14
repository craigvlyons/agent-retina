use crate::{
    Action, ActionId, AgentId, EventId, ExperienceId, IntentId, KnowledgeId, RuleId, SessionId,
    SourceLanguage, StateDeltaKind, TaskId, ToolId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
