use crate::output::{outcome_summary, status_for_outcome};
use crate::{RuntimeTask, RuntimeTaskKind, RuntimeTaskRegistry, RuntimeTaskStore};
use retina_types::{ExecutionControlHandle, Outcome, Result, Task};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;

#[derive(Clone)]
pub struct TaskSupervisor {
    registry: RuntimeTaskRegistry,
    output_dir: PathBuf,
    store: Option<Arc<dyn RuntimeTaskStore>>,
}

impl TaskSupervisor {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            registry: RuntimeTaskRegistry::default(),
            output_dir,
            store: None,
        }
    }

    pub fn with_store(mut self, store: Arc<dyn RuntimeTaskStore>) -> Self {
        self.store = Some(store);
        self
    }

    pub fn registry(&self) -> RuntimeTaskRegistry {
        self.registry.clone()
    }

    pub(crate) fn store(&self) -> Option<&dyn RuntimeTaskStore> {
        self.store.as_deref()
    }

    pub(crate) fn store_arc(&self) -> Option<Arc<dyn RuntimeTaskStore>> {
        self.store.clone()
    }

    pub(crate) fn output_path_for(&self, task: &Task) -> PathBuf {
        self.output_dir.join(format!("{}.output", task.id))
    }

    pub fn spawn(
        &self,
        task: Task,
        kind: RuntimeTaskKind,
        control: ExecutionControlHandle,
        run: impl FnOnce() -> Result<Outcome> + Send + 'static,
    ) -> RuntimeTaskHandle {
        let output_path = Some(self.output_path_for(&task));
        self.registry
            .register(RuntimeTask::new(&task, kind, output_path));
        persist_snapshot(&self.registry, self.store.as_deref(), &task.id);
        self.registry.mark_running(&task.id, "running");
        persist_snapshot(&self.registry, self.store.as_deref(), &task.id);
        let registry = self.registry.clone();
        let store = self.store.clone();
        let task_id = task.id.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let outcome = run();
            let summary = outcome_summary(&outcome);
            registry.append_output(&task_id, &summary);
            registry.mark_terminal(&task_id, status_for_outcome(&outcome), summary);
            persist_snapshot(&registry, store.as_deref(), &task_id);
            let _ = sender.send(outcome);
        });

        RuntimeTaskHandle {
            task,
            control,
            receiver,
            registry: self.registry.clone(),
        }
    }
}

pub(crate) fn persist_snapshot(
    registry: &RuntimeTaskRegistry,
    store: Option<&dyn RuntimeTaskStore>,
    task_id: &retina_types::TaskId,
) {
    let Some(store) = store else {
        return;
    };
    if let Some(task) = registry.snapshot(task_id) {
        let _ = store.save_runtime_task(&task);
    }
}

pub struct RuntimeTaskHandle {
    pub task: Task,
    pub control: ExecutionControlHandle,
    receiver: mpsc::Receiver<Result<Outcome>>,
    registry: RuntimeTaskRegistry,
}

impl RuntimeTaskHandle {
    pub(crate) fn new(
        task: Task,
        control: ExecutionControlHandle,
        receiver: mpsc::Receiver<Result<Outcome>>,
        registry: RuntimeTaskRegistry,
    ) -> Self {
        Self {
            task,
            control,
            receiver,
            registry,
        }
    }

    pub fn recv(self) -> Result<Outcome> {
        let result = self.receiver.recv().map_err(|_| {
            retina_types::KernelError::Execution("runtime task channel disconnected".to_string())
        })?;
        self.registry.mark_notified(&self.task.id);
        result
    }

    pub fn try_recv(&self) -> std::result::Result<Result<Outcome>, mpsc::TryRecvError> {
        let result = self.receiver.try_recv();
        if matches!(result, Ok(_)) {
            self.registry.mark_notified(&self.task.id);
        }
        result
    }

    pub fn snapshot(&self) -> Option<RuntimeTask> {
        self.registry.snapshot(&self.task.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_types::{ActionResult, AgentId, ExecutionControl};
    use tempfile::tempdir;

    #[test]
    fn supervisor_records_terminal_output() {
        let dir = tempdir().unwrap();
        let supervisor = TaskSupervisor::new(dir.path().to_path_buf());
        let control = ExecutionControl::new();
        let task = Task::new(AgentId::new(), "say hi");
        let handle = supervisor.spawn(task, RuntimeTaskKind::Session, control.handle(), || {
            Ok(Outcome::Success(ActionResult::Response {
                message: "hi".to_string(),
            }))
        });
        let result = handle.receiver.recv().unwrap().unwrap();
        assert!(matches!(result, Outcome::Success(_)));
        let snapshot = handle.snapshot().unwrap();
        assert_eq!(snapshot.status, crate::RuntimeTaskStatus::Completed);
        assert!(snapshot.output_path.unwrap().exists());
    }
}
