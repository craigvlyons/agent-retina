use chrono::{DateTime, Utc};
use retina_types::{AgentId, Task, TaskId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeTaskKind {
    Session,
    Command,
    LocalAgent,
    Specialist,
    RemoteAgent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeTaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
    Killed,
}

impl RuntimeTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Blocked | Self::Killed
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeTask {
    pub task_id: TaskId,
    pub parent_task_id: Option<TaskId>,
    pub task_kind: RuntimeTaskKind,
    pub owner_agent_id: AgentId,
    pub status: RuntimeTaskStatus,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub description: String,
    pub prompt_or_objective: String,
    pub output_path: Option<PathBuf>,
    pub output_offset: usize,
    pub progress_summary: Option<String>,
    pub last_activity: DateTime<Utc>,
    pub notified: bool,
}

impl RuntimeTask {
    pub fn new(task: &Task, task_kind: RuntimeTaskKind, output_path: Option<PathBuf>) -> Self {
        let now = Utc::now();
        Self {
            task_id: task.id.clone(),
            parent_task_id: task.parent_task_id.clone(),
            task_kind,
            owner_agent_id: task.agent_id.clone(),
            status: RuntimeTaskStatus::Pending,
            started_at: now,
            ended_at: None,
            description: task.description.clone(),
            prompt_or_objective: task.description.clone(),
            output_path,
            output_offset: 0,
            progress_summary: None,
            last_activity: now,
            notified: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeTaskAttachment {
    pub task_id: TaskId,
    pub task_kind: RuntimeTaskKind,
    pub status: RuntimeTaskStatus,
    pub description: String,
    pub output_path: Option<PathBuf>,
    pub delta_summary: Option<String>,
}

pub trait RuntimeTaskStore: Send + Sync {
    fn save_runtime_task(&self, task: &RuntimeTask) -> retina_types::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_status_terminal_matches_source_task_model() {
        assert!(RuntimeTaskStatus::Completed.is_terminal());
        assert!(RuntimeTaskStatus::Failed.is_terminal());
        assert!(RuntimeTaskStatus::Killed.is_terminal());
        assert!(!RuntimeTaskStatus::Pending.is_terminal());
        assert!(!RuntimeTaskStatus::Running.is_terminal());
    }
}
