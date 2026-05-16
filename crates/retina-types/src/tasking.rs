use crate::{
    Action, ActiveContinuationWindow, AgentId, HashScope, IntentId, RecentContext, RoutingDecision,
    SessionId, TaskId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub session_id: SessionId,
    pub agent_id: AgentId,
    #[serde(default)]
    pub parent_task_id: Option<TaskId>,
    pub description: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub recent_context: Option<RecentContext>,
    #[serde(default)]
    pub resume_context: Option<TaskResumeContext>,
    pub metadata: BTreeMap<String, String>,
}

impl Task {
    pub fn new(agent_id: AgentId, description: impl Into<String>) -> Self {
        Self {
            id: TaskId::new(),
            session_id: crate::SessionId::new(),
            agent_id,
            parent_task_id: None,
            description: description.into(),
            created_at: Utc::now(),
            recent_context: None,
            resume_context: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn spawn_child(
        parent: &Task,
        agent_id: AgentId,
        description: impl Into<String>,
        recent_context: Option<RecentContext>,
    ) -> Self {
        Self {
            id: TaskId::new(),
            session_id: parent.session_id.clone(),
            agent_id,
            parent_task_id: Some(parent.id.clone()),
            description: description.into(),
            created_at: Utc::now(),
            recent_context,
            resume_context: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn resume_from_snapshot(
        agent_id: AgentId,
        snapshot: TaskRecoverySnapshot,
        description: Option<String>,
    ) -> Self {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "resumed_from_task_id".to_string(),
            snapshot.source_task_id.to_string(),
        );
        metadata.insert(
            "resumed_from_session_id".to_string(),
            snapshot.source_session_id.to_string(),
        );
        metadata.insert("resume_reason".to_string(), snapshot.resume_reason.clone());

        Self {
            id: TaskId::new(),
            session_id: snapshot.source_session_id.clone(),
            agent_id,
            parent_task_id: Some(snapshot.source_task_id.clone()),
            description: description.unwrap_or_else(|| snapshot.objective.clone()),
            created_at: Utc::now(),
            recent_context: Some(snapshot.derived_recent_context()),
            resume_context: Some(TaskResumeContext::from_snapshot(snapshot)),
            metadata,
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
pub struct SpawnAgentRequest {
    pub parent_task: Task,
    pub prompt: String,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteAgentRequest {
    pub parent_task: Task,
    pub decision: RoutingDecision,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResumeContext {
    pub source_task_id: TaskId,
    pub source_session_id: SessionId,
    pub objective: String,
    pub continuation_window: ActiveContinuationWindow,
    pub resume_reason: String,
}

impl TaskResumeContext {
    pub fn from_snapshot(snapshot: TaskRecoverySnapshot) -> Self {
        Self {
            source_task_id: snapshot.source_task_id,
            source_session_id: snapshot.source_session_id,
            objective: snapshot.objective,
            continuation_window: snapshot.continuation_window,
            resume_reason: snapshot.resume_reason,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecoverySnapshot {
    pub source_task_id: TaskId,
    pub source_session_id: SessionId,
    pub source_agent_id: AgentId,
    pub objective: String,
    #[serde(default)]
    pub continuation_window: ActiveContinuationWindow,
    #[serde(default)]
    pub recent_context: Option<RecentContext>,
    pub resume_reason: String,
}

impl TaskRecoverySnapshot {
    pub fn from_live_state(
        task: &Task,
        continuation_window: ActiveContinuationWindow,
        resume_reason: impl Into<String>,
    ) -> Self {
        Self {
            source_task_id: task.id.clone(),
            source_session_id: task.session_id.clone(),
            source_agent_id: task.agent_id.clone(),
            objective: task.description.clone(),
            continuation_window,
            recent_context: task.recent_context.clone(),
            resume_reason: resume_reason.into(),
        }
    }

    pub fn derived_recent_context(&self) -> RecentContext {
        self.recent_context
            .clone()
            .unwrap_or_else(|| RecentContext {
                prior_objective: self.objective.clone(),
                prior_answer_summary: Some(self.resume_reason.clone()),
                sources: self.continuation_window.reannounced_sources.clone(),
                artifacts: self.continuation_window.reannounced_artifacts.clone(),
            })
    }
}
