use crate::chat::StreamingMemory;
use crate::runtime::{open_memory, root_db_path};
use retina_kernel::Kernel;
use retina_llm_claude::ClaudeReasoner;
use retina_memory_sqlite::{MemoryStats, SqliteMemory};
use retina_shell_cli::CliShell;
use retina_traits::Memory;
use retina_types::*;

pub struct AgentController {
    kernel: Kernel,
}

impl AgentController {
    pub fn new(stream_events: bool) -> Result<Self> {
        let memory = open_memory(root_db_path()?)?;
        let kernel = if stream_events {
            Kernel::new(
                Box::new(CliShell::new()),
                Box::new(ClaudeReasoner::new()),
                Box::new(StreamingMemory::new(memory)),
            )?
        } else {
            Kernel::new(
                Box::new(CliShell::new()),
                Box::new(ClaudeReasoner::new()),
                Box::new(memory),
            )?
        };
        Ok(Self { kernel })
    }

    pub fn execute_task(&self, task_description: impl Into<String>) -> Result<Outcome> {
        let task = Task::new(AgentId("root".to_string()), task_description.into());
        self.kernel.execute_task(task)
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
