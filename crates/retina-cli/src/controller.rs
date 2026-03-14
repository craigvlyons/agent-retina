use crate::chat::StreamingMemory;
use crate::runtime::{open_memory, root_agent_id, root_db_path, root_manifest};
use retina_kernel::Kernel;
use retina_llm_claude::ClaudeReasoner;
use retina_memory_sqlite::{MemoryStats, SqliteMemory};
use retina_shell_cli::{CliShell, ScopedShell};
use retina_traits::{Memory, Shell};
use retina_types::*;
use std::sync::{atomic::AtomicBool, Arc};

pub struct AgentController {
    kernel: Kernel,
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
        let manifest = memory
            .load_manifest(&root_agent_id())?
            .unwrap_or(root_manifest()?);
        let kernel = if stream_events {
            Kernel::new(
                Box::new(ScopedShell::new(CliShell::new(), manifest.authority.clone())),
                Box::new(ClaudeReasoner::new()),
                Box::new(StreamingMemory::new(
                    memory,
                    debug_events.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                )),
            )?
        } else {
            Kernel::new(
                Box::new(ScopedShell::new(CliShell::new(), manifest.authority.clone())),
                Box::new(ClaudeReasoner::new()),
                Box::new(memory),
            )?
        };
        Ok(Self { kernel })
    }

    pub fn new_with_streaming_and_shell(
        shell: Box<dyn Shell>,
        debug_events: Arc<AtomicBool>,
    ) -> Result<Self> {
        let memory = open_memory(root_db_path()?)?;
        let kernel = Kernel::new(
            shell,
            Box::new(ClaudeReasoner::new()),
            Box::new(StreamingMemory::new(memory, debug_events)),
        )?;
        Ok(Self { kernel })
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
        self.kernel.execute_task_with_config(task, config)
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
}
