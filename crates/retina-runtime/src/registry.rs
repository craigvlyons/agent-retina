use crate::output::{append_output_file, compact_summary, read_output_delta};
use crate::{RuntimeTask, RuntimeTaskAttachment, RuntimeTaskStatus, TaskOutputDelta};
use chrono::{DateTime, Utc};
use retina_types::TaskId;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, Default)]
pub struct RuntimeTaskRegistry {
    inner: Arc<Mutex<BTreeMap<TaskId, RuntimeTask>>>,
}

impl RuntimeTaskRegistry {
    pub fn register(&self, task: RuntimeTask) {
        recover_mutex(&self.inner).insert(task.task_id.clone(), task);
    }

    pub fn mark_running(&self, task_id: &TaskId, progress: impl Into<String>) {
        self.update(task_id, |task| {
            task.status = RuntimeTaskStatus::Running;
            task.progress_summary = Some(progress.into());
            task.last_activity = Utc::now();
        });
    }

    pub fn mark_terminal(
        &self,
        task_id: &TaskId,
        status: RuntimeTaskStatus,
        progress: impl Into<String>,
    ) {
        self.mark_terminal_at(task_id, status, progress.into(), Utc::now());
    }

    pub(crate) fn mark_terminal_at(
        &self,
        task_id: &TaskId,
        status: RuntimeTaskStatus,
        progress: String,
        timestamp: DateTime<Utc>,
    ) {
        self.update(task_id, |task| {
            task.status = status;
            task.progress_summary = Some(progress);
            task.ended_at = Some(timestamp);
            task.last_activity = timestamp;
        });
    }

    pub fn append_output(&self, task_id: &TaskId, content: &str) {
        self.update(task_id, |task| {
            task.output_offset += content.len();
            task.progress_summary = Some(compact_summary(content));
            task.last_activity = Utc::now();
            if let Some(path) = &task.output_path {
                let _ = append_output_file(path, content);
            }
        });
    }

    pub fn mark_notified(&self, task_id: &TaskId) {
        self.update(task_id, |task| {
            task.notified = true;
            task.last_activity = Utc::now();
        });
    }

    pub fn snapshot(&self, task_id: &TaskId) -> Option<RuntimeTask> {
        recover_mutex(&self.inner).get(task_id).cloned()
    }

    pub fn snapshots(&self) -> Vec<RuntimeTask> {
        let mut tasks = recover_mutex(&self.inner)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| right.last_activity.cmp(&left.last_activity));
        tasks
    }

    pub fn running(&self) -> Vec<RuntimeTask> {
        self.snapshots()
            .into_iter()
            .filter(|task| task.status == RuntimeTaskStatus::Running)
            .collect()
    }

    pub fn attachments(&self) -> Vec<RuntimeTaskAttachment> {
        self.snapshots()
            .into_iter()
            .filter(|task| !task.notified)
            .map(|task| RuntimeTaskAttachment {
                task_id: task.task_id,
                task_kind: task.task_kind,
                status: task.status,
                description: task.description,
                output_path: task.output_path,
                delta_summary: task.progress_summary,
            })
            .collect()
    }

    pub fn output_delta(
        &self,
        task_id: &TaskId,
        max_bytes: usize,
    ) -> std::io::Result<Option<TaskOutputDelta>> {
        let Some(task) = self.snapshot(task_id) else {
            return Ok(None);
        };
        let Some(path) = task.output_path else {
            return Ok(None);
        };
        let delta = read_output_delta(&path, task.output_offset, max_bytes)?;
        let new_offset = delta.new_offset;
        self.update(task_id, |task| {
            task.output_offset = new_offset;
            task.last_activity = Utc::now();
        });
        Ok(Some(delta))
    }

    fn update(&self, task_id: &TaskId, updater: impl FnOnce(&mut RuntimeTask)) {
        if let Some(task) = recover_mutex(&self.inner).get_mut(task_id) {
            updater(task);
        }
    }
}

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimeTaskKind;
    use crate::output::append_output_file;
    use retina_types::{AgentId, Task};
    use tempfile::tempdir;

    #[test]
    fn output_delta_advances_registry_offset() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("task.output");
        let task = Task::new(AgentId::new(), "watch output");
        let task_id = task.id.clone();
        let registry = RuntimeTaskRegistry::default();
        registry.register(RuntimeTask::new(
            &task,
            RuntimeTaskKind::Session,
            Some(output_path.clone()),
        ));
        append_output_file(&output_path, "first").unwrap();
        append_output_file(&output_path, "second").unwrap();

        let first = registry.output_delta(&task_id, 6).unwrap().unwrap();
        assert_eq!(first.content, "first\n");
        assert_eq!(registry.snapshot(&task_id).unwrap().output_offset, 6);

        let second = registry.output_delta(&task_id, 64).unwrap().unwrap();
        assert_eq!(second.content, "second\n");
    }
}
