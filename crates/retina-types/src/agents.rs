use crate::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
pub struct AgentMessage {
    pub from: AgentId,
    pub to: AgentId,
    pub kind: MessageKind,
    pub payload: serde_json::Value,
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
