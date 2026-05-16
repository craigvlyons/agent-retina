use crate::{Action, AgentId, HashScope, IntentId, RecentContext, RoutingDecision, TaskId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub session_id: crate::SessionId,
    pub agent_id: AgentId,
    #[serde(default)]
    pub parent_task_id: Option<TaskId>,
    pub description: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub recent_context: Option<RecentContext>,
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
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intent {
    pub id: IntentId,
    pub task_id: TaskId,
    pub session_id: crate::SessionId,
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
