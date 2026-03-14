use crate::chat::StreamingMemory;
use crate::runtime::{open_memory, retina_home, root_agent_id, root_db_path, root_manifest};
use retina_kernel::Kernel;
use retina_llm_claude::ClaudeReasoner;
use retina_memory_sqlite::{MemoryStats, SqliteMemory};
use retina_shell_cli::{CliShell, ScopedShell};
use retina_traits::{Memory, Shell};
use retina_types::*;
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use std::thread;

pub struct AgentController {
    kernel: Arc<Kernel>,
}

pub struct RunningTask {
    pub task: Task,
    pub control: ExecutionControlHandle,
    receiver: mpsc::Receiver<Result<Outcome>>,
}

impl AgentController {
    pub fn new(stream_events: bool) -> Result<Self> {
        self::AgentController::new_with_optional_stream(stream_events, None)
    }

    fn new_with_optional_stream(
        stream_events: bool,
        debug_events: Option<Arc<AtomicBool>>,
    ) -> Result<Self> {
        let memory = open_memory(root_db_path()?)?;
        let registry = memory.agent_registry()?;
        let manifest = memory
            .load_manifest(&root_agent_id())?
            .unwrap_or(root_manifest()?);
        let kernel = if stream_events {
            Kernel::new_with_registry(
                Box::new(ScopedShell::new(
                    CliShell::new(),
                    manifest.authority.clone(),
                )),
                Box::new(ClaudeReasoner::new()),
                Box::new(StreamingMemory::new(
                    memory,
                    debug_events.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                )),
                registry,
            )?
        } else {
            Kernel::new_with_registry(
                Box::new(ScopedShell::new(
                    CliShell::new(),
                    manifest.authority.clone(),
                )),
                Box::new(ClaudeReasoner::new()),
                Box::new(memory),
                registry,
            )?
        };
        Ok(Self {
            kernel: Arc::new(kernel),
        })
    }

    pub fn new_with_streaming_and_shell(
        shell: Box<dyn Shell>,
        debug_events: Arc<AtomicBool>,
    ) -> Result<Self> {
        let memory = open_memory(root_db_path()?)?;
        let registry = memory.agent_registry()?;
        let kernel = Kernel::new_with_registry(
            shell,
            Box::new(ClaudeReasoner::new()),
            Box::new(StreamingMemory::new(memory, debug_events)),
            registry,
        )?;
        Ok(Self {
            kernel: Arc::new(kernel),
        })
    }

    pub fn execute_task(&self, task_description: impl Into<String>) -> Result<Outcome> {
        self.execute_task_with_config(task_description, ExecutionConfig::default())
    }

    pub fn execute_task_with_config(
        &self,
        task_description: impl Into<String>,
        config: ExecutionConfig,
    ) -> Result<Outcome> {
        let task = Task::new(root_agent_id(), task_description.into());
        update_root_manifest_state(
            AgentStatus::Running,
            AgentLifecyclePhase::Busy,
            Some("executing task"),
        )?;
        let outcome = self.kernel.execute_task_with_config(task, config);
        update_root_manifest_state(
            AgentStatus::Idle,
            AgentLifecyclePhase::CoolingDown,
            Some("waiting for next task"),
        )?;
        outcome
    }

    pub fn spawn_task(
        &self,
        task_description: impl Into<String>,
        mut config: ExecutionConfig,
    ) -> RunningTask {
        let task = Task::new(root_agent_id(), task_description.into());
        let control = ExecutionControl::new();
        let control_handle = control.handle();
        config.control = Some(control_handle.clone());
        let kernel = Arc::clone(&self.kernel);
        let task_for_thread = task.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = update_root_manifest_state(
                AgentStatus::Running,
                AgentLifecyclePhase::Busy,
                Some("executing task"),
            );
            let outcome = kernel.execute_task_with_config(task_for_thread, config);
            let _ = update_root_manifest_state(
                AgentStatus::Idle,
                AgentLifecyclePhase::CoolingDown,
                Some("waiting for next task"),
            );
            let _ = sender.send(outcome);
        });
        RunningTask {
            task,
            control: control_handle,
            receiver,
        }
    }
}

impl RunningTask {
    pub fn try_recv(&self) -> std::result::Result<Result<Outcome>, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

pub struct InspectController {
    memory: SqliteMemory,
}

impl InspectController {
    pub fn new() -> Result<Self> {
        Ok(Self {
            memory: open_memory(root_db_path()?)?,
        })
    }

    pub fn recent_timeline(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        self.memory.recent_states(limit)
    }

    pub fn memory_lookup(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<(Vec<KnowledgeNode>, Vec<Experience>)> {
        Ok((
            self.memory.recall_knowledge(query, limit)?,
            self.memory.recall_experiences(query, limit)?,
        ))
    }

    pub fn stats(&self) -> Result<MemoryStats> {
        self.memory.stats()
    }

    pub fn agent_registry(&self) -> Result<AgentRegistrySnapshot> {
        self.memory.agent_registry()
    }

    pub fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        self.memory.append_timeline_event(event)
    }
}

fn update_root_manifest_state(
    status: AgentStatus,
    phase: AgentLifecyclePhase,
    reason: Option<&str>,
) -> Result<()> {
    let memory = open_memory(root_db_path()?)?;
    if let Some(manifest) =
        memory.update_manifest_lifecycle(&root_agent_id(), status, phase, reason)?
    {
        write_root_manifest_file(&manifest)?;
    }
    Ok(())
}

fn write_root_manifest_file(manifest: &AgentManifest) -> Result<()> {
    let path = retina_home()?.join("root").join("manifest.toml");
    retina_memory_sqlite::write_manifest(path, manifest)
}
