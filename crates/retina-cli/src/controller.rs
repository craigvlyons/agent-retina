use crate::chat::StreamingMemory;
use crate::runtime::{
    normalize_root_manifest, open_memory, retina_home, root_agent_id, root_db_path, root_manifest,
    root_task_output_dir,
};
use retina_kernel::Kernel;
use retina_llm_claude::{ClaudeReasoner, ClaudeRuntimeConfigSnapshot};
use retina_mcp_client::{ConfiguredMcpRuntime, default_config_path};
use retina_memory_sqlite::{MemoryStats, SqliteMemory};
use retina_runtime::{RuntimeTask, RuntimeTaskKind, RuntimeTaskRegistry, TaskSupervisor};
use retina_shell_cli::{CliShell, ScopedShell};
use retina_tools::ToolPolicy;
use retina_traits::{McpRuntime, Memory, Shell};
use retina_transport_local::{LocalAgentRuntimeService, LocalTransportConfig};
use retina_types::*;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

pub struct AgentController {
    kernel: Arc<Kernel>,
    supervisor: TaskSupervisor,
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
        let manifest = normalize_root_manifest(
            memory
                .load_manifest(&root_agent_id())?
                .unwrap_or(root_manifest()?),
        );
        let supervisor = TaskSupervisor::new(root_task_output_dir()?)
            .with_store(Arc::new(open_memory(root_db_path()?)?));
        let agent_runtime = Arc::new(LocalAgentRuntimeService::new(
            supervisor.clone(),
            local_transport_config()?,
            manifest.authority.clone(),
        ));
        let mcp_runtime = Arc::new(ConfiguredMcpRuntime::new(default_config_path(
            &retina_home()?,
        )));
        let kernel = if stream_events {
            Kernel::new_with_runtime(
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
                ToolPolicy::from_authority(&manifest.authority),
                Some(agent_runtime.clone()),
                Some(mcp_runtime.clone()),
            )?
        } else {
            Kernel::new_with_runtime(
                Box::new(ScopedShell::new(
                    CliShell::new(),
                    manifest.authority.clone(),
                )),
                Box::new(ClaudeReasoner::new()),
                Box::new(memory),
                registry,
                ToolPolicy::from_authority(&manifest.authority),
                Some(agent_runtime),
                Some(mcp_runtime),
            )?
        };
        Ok(Self {
            kernel: Arc::new(kernel),
            supervisor,
        })
    }

    pub fn new_with_streaming_and_shell(
        shell: Box<dyn Shell>,
        authority: AgentAuthority,
        debug_events: Arc<AtomicBool>,
    ) -> Result<Self> {
        let memory = open_memory(root_db_path()?)?;
        let registry = memory.agent_registry()?;
        let supervisor = TaskSupervisor::new(root_task_output_dir()?)
            .with_store(Arc::new(open_memory(root_db_path()?)?));
        let agent_runtime = Arc::new(LocalAgentRuntimeService::new(
            supervisor.clone(),
            local_transport_config()?,
            authority.clone(),
        ));
        let mcp_runtime = Arc::new(ConfiguredMcpRuntime::new(default_config_path(
            &retina_home()?,
        )));
        let kernel = Kernel::new_with_runtime(
            shell,
            Box::new(ClaudeReasoner::new()),
            Box::new(StreamingMemory::new(memory, debug_events)),
            registry,
            ToolPolicy::from_authority(&authority),
            Some(agent_runtime),
            Some(mcp_runtime),
        )?;
        Ok(Self {
            kernel: Arc::new(kernel),
            supervisor,
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
        self.spawn_task(task_description, config).recv()
    }

    pub fn spawn_task(
        &self,
        task_description: impl Into<String>,
        mut config: ExecutionConfig,
    ) -> retina_runtime::RuntimeTaskHandle {
        let task_description = task_description.into();
        let task = build_task_for_description(task_description.clone())
            .unwrap_or_else(|_| Task::new(root_agent_id(), task_description));
        let control = ExecutionControl::new();
        let control_handle = control.handle();
        config.control = Some(control_handle.clone());
        let kernel = Arc::clone(&self.kernel);
        let task_for_thread = task.clone();
        self.supervisor
            .spawn(task, RuntimeTaskKind::Session, control_handle, move || {
                let _ = update_root_manifest_state(
                    AgentStatus::Running,
                    AgentLifecyclePhase::Busy,
                    Some("executing task"),
                );
                let outcome = run_task_catching_panics(&kernel, task_for_thread, config);
                let _ = update_root_manifest_state(
                    AgentStatus::Idle,
                    AgentLifecyclePhase::CoolingDown,
                    Some("waiting for next task"),
                );
                outcome
            })
    }

    pub fn spawn_resumed_task(
        &self,
        snapshot: TaskRecoverySnapshot,
        mut config: ExecutionConfig,
    ) -> retina_runtime::RuntimeTaskHandle {
        let task = Task::resume_from_snapshot(root_agent_id(), snapshot, None);
        let control = ExecutionControl::new();
        let control_handle = control.handle();
        config.control = Some(control_handle.clone());
        let kernel = Arc::clone(&self.kernel);
        let task_for_thread = task.clone();
        self.supervisor
            .spawn(task, RuntimeTaskKind::Session, control_handle, move || {
                let _ = update_root_manifest_state(
                    AgentStatus::Running,
                    AgentLifecyclePhase::Busy,
                    Some("resuming task"),
                );
                let outcome = run_task_catching_panics(&kernel, task_for_thread, config);
                let _ = update_root_manifest_state(
                    AgentStatus::Idle,
                    AgentLifecyclePhase::CoolingDown,
                    Some("waiting for next task"),
                );
                outcome
            })
    }
}

fn build_task_for_description(task_description: String) -> Result<Task> {
    Ok(Task::new(root_agent_id(), task_description))
}

fn local_transport_config() -> Result<LocalTransportConfig> {
    let home = retina_home()?;
    Ok(LocalTransportConfig {
        db_path: root_db_path()?,
        agents_dir: home.join("agents"),
        retina_home: home,
    })
}

pub struct InspectController {
    memory: SqliteMemory,
}

pub struct WorkerOverview {
    pub manifest: AgentManifest,
    pub stats: MemoryStats,
    pub db_path: PathBuf,
    pub db_size_bytes: u64,
    pub terminal_tasks: TerminalTaskStats,
    pub active_rules: Vec<ReflexiveRule>,
    pub compaction_stats: CompactionStats,
    pub claude_runtime: ClaudeRuntimeConfigSnapshot,
    pub runtime_tasks: Vec<RuntimeTask>,
}

pub struct TerminalTaskStats {
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub blocked: usize,
}

pub struct CompactionStats {
    pub compaction_events: usize,
    pub cache_reads: u64,
    pub cache_creations: u64,
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

    pub fn latest_task_projection(&self) -> Result<Option<TaskState>> {
        let events = self.memory.recent_states(200)?;
        for event in events {
            if let Some(value) = event.payload_json.get("continuation_window") {
                let continuation_window =
                    serde_json::from_value::<ActiveContinuationWindow>(value.clone())
                        .map_err(|error| KernelError::Storage(error.to_string()))?;
                return Ok(Some(continuation_window.project_task_state()));
            }
        }
        Ok(None)
    }

    pub fn latest_continuation_window(&self) -> Result<Option<ActiveContinuationWindow>> {
        let events = self.memory.recent_states(200)?;
        for event in events {
            if let Some(value) = event.payload_json.get("continuation_window") {
                let continuation_window =
                    serde_json::from_value::<ActiveContinuationWindow>(value.clone())
                        .map_err(|error| KernelError::Storage(error.to_string()))?;
                return Ok(Some(continuation_window));
            }
        }
        Ok(None)
    }

    pub fn latest_recoverable_task_snapshot(&self) -> Result<Option<TaskRecoverySnapshot>> {
        self.recoverable_task_snapshot_for(None)
    }

    pub fn recoverable_task_snapshot_by_id(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskRecoverySnapshot>> {
        self.recoverable_task_snapshot_for(Some(task_id))
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

    pub fn recent_runtime_tasks(&self, limit: usize) -> Result<Vec<RuntimeTask>> {
        let tasks = self.memory.recent_runtime_tasks(limit)?;
        if !tasks.is_empty() {
            return Ok(tasks);
        }
        let events = self.memory.recent_states(500)?;
        let mut tasks = RuntimeTaskRegistry::from_timeline(&events).snapshots();
        tasks.truncate(limit);
        Ok(tasks)
    }

    pub fn worker_overview(&self) -> Result<WorkerOverview> {
        let db_path = root_db_path()?;
        let manifest = normalize_root_manifest(
            self.memory
                .load_manifest(&root_agent_id())?
                .unwrap_or(root_manifest()?),
        );
        let stats = self.memory.stats()?;
        let terminal_tasks = summarize_terminal_tasks(&self.memory.recent_states(200)?);
        let recent_events = self.memory.recent_states(500)?;
        let active_rules = self.memory.active_rules()?;
        let db_size_bytes = std::fs::metadata(&db_path)
            .map(|meta| meta.len())
            .unwrap_or(0);

        Ok(WorkerOverview {
            manifest,
            stats,
            db_path,
            db_size_bytes,
            terminal_tasks,
            active_rules,
            compaction_stats: summarize_compaction_stats(&recent_events),
            claude_runtime: ClaudeReasoner::runtime_config_snapshot(),
            runtime_tasks: {
                let mut tasks = self.memory.recent_runtime_tasks(10)?;
                if tasks.is_empty() {
                    tasks = RuntimeTaskRegistry::from_timeline(&recent_events)
                        .snapshots()
                        .into_iter()
                        .take(10)
                        .collect();
                }
                tasks
            },
        })
    }

    pub fn cleanup_memory(&self, config: ConsolidationConfig) -> Result<ConsolidationReport> {
        self.memory.consolidate(&config)
    }

    pub fn mcp_snapshot(&self) -> Result<McpRegistrySnapshot> {
        Ok(ConfiguredMcpRuntime::new(default_config_path(&retina_home()?)).snapshot()?)
    }

    pub fn agent_registry(&self) -> Result<AgentRegistrySnapshot> {
        self.memory.agent_registry()
    }

    pub fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        self.memory.append_timeline_event(event)
    }

    fn recoverable_task_snapshot_for(
        &self,
        task_id: Option<&str>,
    ) -> Result<Option<TaskRecoverySnapshot>> {
        let events = self.memory.recent_states(2_000)?;
        for event in events {
            if !matches!(
                event.event_type,
                TimelineEventType::TaskBlocked | TimelineEventType::TaskFailed
            ) {
                continue;
            }
            if task_id.is_some_and(|expected| event.task_id.0 != expected) {
                continue;
            }
            let Some(value) = event.payload_json.get("recovery_snapshot") else {
                continue;
            };
            let snapshot = serde_json::from_value::<TaskRecoverySnapshot>(value.clone())
                .map_err(|error| KernelError::Storage(error.to_string()))?;
            return Ok(Some(snapshot));
        }
        Ok(None)
    }
}

fn run_task_catching_panics(
    kernel: &Kernel,
    task: Task,
    config: ExecutionConfig,
) -> Result<Outcome> {
    panic::catch_unwind(AssertUnwindSafe(|| {
        kernel.execute_task_with_config(task, config)
    }))
    .unwrap_or_else(|panic_payload| Err(task_panic_error(panic_payload)))
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

fn task_panic_error(panic_payload: Box<dyn std::any::Any + Send>) -> KernelError {
    let message = if let Some(text) = panic_payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic_payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic payload".to_string()
    };
    KernelError::Execution(format!("task execution panicked: {message}"))
}

fn summarize_terminal_tasks(events: &[TimelineEvent]) -> TerminalTaskStats {
    let mut stats = TerminalTaskStats {
        completed: 0,
        failed: 0,
        cancelled: 0,
        blocked: 0,
    };

    for event in events {
        match event.event_type {
            TimelineEventType::TaskCompleted => stats.completed += 1,
            TimelineEventType::TaskFailed => stats.failed += 1,
            TimelineEventType::TaskCancelled => stats.cancelled += 1,
            TimelineEventType::TaskBlocked => stats.blocked += 1,
            _ => {
                if event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .map(|reason| reason.contains("blocked"))
                    .unwrap_or(false)
                {
                    stats.blocked += 1;
                }
            }
        }
    }

    stats
}

fn summarize_compaction_stats(events: &[TimelineEvent]) -> CompactionStats {
    let mut stats = CompactionStats {
        compaction_events: 0,
        cache_reads: 0,
        cache_creations: 0,
    };

    for event in events {
        if matches!(event.event_type, TimelineEventType::TaskCompacted) {
            stats.compaction_events += 1;
        }
        if let Some(tokens) = event
            .payload_json
            .get("tokens")
            .and_then(|value| serde_json::from_value::<TokenUsage>(value.clone()).ok())
        {
            stats.cache_reads += tokens.cache_read_input_tokens as u64;
            stats.cache_creations += tokens.cache_creation_input_tokens as u64;
        }
    }

    stats
}
