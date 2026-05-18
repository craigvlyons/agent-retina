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

    pub fn spawn_follow_up_task(
        &self,
        seed: TaskFollowUpSeed,
        task_description: impl Into<String>,
        mut config: ExecutionConfig,
    ) -> retina_runtime::RuntimeTaskHandle {
        let task_description = task_description.into();
        let task = Task::follow_up_from_seed(root_agent_id(), seed, task_description);
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
                    Some("executing follow-up task"),
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
    pub recovery_stats: RecoveryStats,
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

pub struct RecoveryStats {
    pub total: usize,
    pub max_output_tokens_escalate: usize,
    pub max_output_tokens_recovery: usize,
    pub prompt_too_long_compaction: usize,
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

    pub fn latest_follow_up_seed(&self) -> Result<Option<TaskFollowUpSeed>> {
        reconstruct_follow_up_seed_from_events(&self.memory.recent_states(2_000)?, None)
    }

    pub fn latest_task_projection(&self) -> Result<Option<TaskState>> {
        let events = self.memory.recent_states(200)?;
        Ok(
            reconstruct_latest_continuation_window_from_events(&events, None)?
                .map(|(_, window)| window.project_task_state()),
        )
    }

    pub fn latest_continuation_window(&self) -> Result<Option<ActiveContinuationWindow>> {
        let events = self.memory.recent_states(200)?;
        Ok(
            reconstruct_latest_continuation_window_from_events(&events, None)?
                .map(|(_, window)| window),
        )
    }

    pub fn follow_up_seed_for_task(&self, task_id: &TaskId) -> Result<Option<TaskFollowUpSeed>> {
        reconstruct_follow_up_seed_from_events(&self.memory.recent_states(2_000)?, Some(task_id))
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
            recovery_stats: summarize_recovery_stats(&recent_events),
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
        for event in &events {
            if !matches!(
                event.event_type,
                TimelineEventType::TaskBlocked | TimelineEventType::TaskFailed
            ) {
                continue;
            }
            if task_id.is_some_and(|expected| event.task_id.0 != expected) {
                continue;
            }
            let Some((_, continuation_window)) =
                reconstruct_latest_continuation_window_from_events(&events, Some(&event.task_id))?
            else {
                continue;
            };
            let resume_reason = event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                .unwrap_or("recoverable task state")
                .to_string();
            return Ok(Some(TaskRecoverySnapshot {
                source_task_id: event.task_id.clone(),
                source_session_id: event.session_id.clone(),
                source_agent_id: event.agent_id.clone(),
                objective: continuation_window.objective.clone(),
                continuation_window,
                resume_reason,
            }));
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

fn reconstruct_follow_up_seed_from_events(
    events: &[TimelineEvent],
    task_id: Option<&TaskId>,
) -> Result<Option<TaskFollowUpSeed>> {
    let mut ordered = events.to_vec();
    ordered.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));

    let terminal_event = ordered.iter().find(|event| {
        matches!(
            event.event_type,
            TimelineEventType::TaskCompleted
                | TimelineEventType::TaskBlocked
                | TimelineEventType::TaskFailed
        ) && task_id
            .map(|expected| event.task_id == *expected)
            .unwrap_or(true)
    });
    let Some(terminal_event) = terminal_event else {
        return Ok(None);
    };

    let Some((_, continuation_window)) = reconstruct_latest_continuation_window_from_events(
        &ordered,
        Some(&terminal_event.task_id),
    )?
    else {
        return Ok(None);
    };

    Ok(Some(TaskFollowUpSeed {
        source_task_id: terminal_event.task_id.clone(),
        source_session_id: terminal_event.session_id.clone(),
        source_agent_id: terminal_event.agent_id.clone(),
        objective: continuation_window.objective.clone(),
        recent_context: Some(RecentContext {
            prior_objective: continuation_window.objective.clone(),
            prior_answer_summary: terminal_event_follow_up_summary(terminal_event),
            sticky_constraints: Vec::new(),
            sources: continuation_window.transcript.reduced_working_sources(),
            artifacts: continuation_window.transcript.reduced_artifact_references(),
        }),
        continuation_window,
    }))
}

fn reconstruct_latest_continuation_window_from_events(
    events: &[TimelineEvent],
    task_id: Option<&TaskId>,
) -> Result<Option<(TaskId, ActiveContinuationWindow)>> {
    let mut ordered = events.to_vec();
    ordered.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));

    let Some(event) = ordered.iter().find(|event| {
        task_id
            .map(|expected| event.task_id == *expected)
            .unwrap_or(true)
            && event.payload_json.get("continuation_window").is_some()
    }) else {
        return Ok(None);
    };

    let value = event
        .payload_json
        .get("continuation_window")
        .cloned()
        .expect("continuation window checked above");
    let mut continuation_window = serde_json::from_value::<ActiveContinuationWindow>(value)
        .map_err(|error| KernelError::Storage(error.to_string()))?;
    let explicit_replacements = collect_content_replacements_from_events(&ordered, &event.task_id)?;
    if !explicit_replacements.is_empty() {
        continuation_window.content_replacements = ContentReplacementState {
            records: explicit_replacements,
        };
    }
    let reconstructed_read_state_cache =
        collect_read_state_cache_from_events(&ordered, &event.task_id)?;
    if !reconstructed_read_state_cache.is_empty() {
        continuation_window.read_state_cache = reconstructed_read_state_cache;
    }
    let reconstructed_search_state_cache =
        collect_search_state_cache_from_events(&ordered, &event.task_id)?;
    if !reconstructed_search_state_cache.is_empty() {
        continuation_window.search_state_cache = reconstructed_search_state_cache;
    }

    Ok(Some((event.task_id.clone(), continuation_window)))
}

fn collect_content_replacements_from_events(
    events: &[TimelineEvent],
    task_id: &TaskId,
) -> Result<Vec<ContentReplacementRecord>> {
    let mut ordered = events
        .iter()
        .filter(|event| {
            event.task_id == *task_id
                && matches!(
                    event.event_type,
                    TimelineEventType::ContentReplacementsRecorded
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));

    let mut records = Vec::new();
    for event in ordered {
        let Some(value) = event.payload_json.get("records") else {
            continue;
        };
        let parsed = serde_json::from_value::<Vec<ContentReplacementRecord>>(value.clone())
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        for record in parsed {
            if records.iter().any(|existing: &ContentReplacementRecord| {
                existing.replacement_id == record.replacement_id
            }) {
                continue;
            }
            records.push(record);
        }
    }

    Ok(records)
}

fn collect_read_state_cache_from_events(
    events: &[TimelineEvent],
    task_id: &TaskId,
) -> Result<Vec<CachedFileReadState>> {
    let mut ordered = events
        .iter()
        .filter(|event| {
            event.task_id == *task_id
                && matches!(event.event_type, TimelineEventType::ActionResultReceived)
        })
        .cloned()
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));

    let mut states = Vec::<CachedFileReadState>::new();
    for event in ordered {
        let Some(value) = event.payload_json.get("result") else {
            continue;
        };
        let result = serde_json::from_value::<ActionResult>(value.clone())
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        let Some(next_state) = CachedFileReadState::from_action_result(&result) else {
            continue;
        };
        if let Some(existing) = states
            .iter_mut()
            .find(|state| state.path == next_state.path)
        {
            *existing = next_state;
        } else {
            states.push(next_state);
        }
    }

    Ok(states)
}

fn collect_search_state_cache_from_events(
    events: &[TimelineEvent],
    task_id: &TaskId,
) -> Result<Vec<CachedSearchState>> {
    let mut ordered = events
        .iter()
        .filter(|event| {
            event.task_id == *task_id
                && matches!(event.event_type, TimelineEventType::ActionResultReceived)
        })
        .cloned()
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));

    let mut states = Vec::<CachedSearchState>::new();
    for event in ordered {
        let Some(value) = event.payload_json.get("result") else {
            continue;
        };
        let result = serde_json::from_value::<ActionResult>(value.clone())
            .map_err(|error| KernelError::Storage(error.to_string()))?;
        let Some(next_state) = CachedSearchState::from_action_result(&result) else {
            continue;
        };
        let next_key = next_state.cache_key();
        if let Some(existing) = states
            .iter_mut()
            .find(|state| state.cache_key() == next_key)
        {
            *existing = next_state;
        } else {
            states.push(next_state);
        }
    }

    Ok(states)
}

fn terminal_event_follow_up_summary(event: &TimelineEvent) -> Option<String> {
    event
        .payload_json
        .get("final_answer_summary")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            event
                .payload_json
                .get("outcome")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
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
            _ => {}
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

fn summarize_recovery_stats(events: &[TimelineEvent]) -> RecoveryStats {
    let mut stats = RecoveryStats {
        total: 0,
        max_output_tokens_escalate: 0,
        max_output_tokens_recovery: 0,
        prompt_too_long_compaction: 0,
    };

    for event in events {
        if !matches!(event.event_type, TimelineEventType::TaskRecoveryContinued) {
            continue;
        }
        stats.total += 1;
        match event
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str())
        {
            Some("max_output_tokens_escalate") => stats.max_output_tokens_escalate += 1,
            Some("max_output_tokens_recovery") => stats.max_output_tokens_recovery += 1,
            Some("prompt_too_long_compaction") => stats.prompt_too_long_compaction += 1,
            _ => {}
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;

    fn sample_continuation(objective: &str) -> ActiveContinuationWindow {
        ActiveContinuationWindow {
            objective: objective.to_string(),
            current_step: 3,
            max_steps: 10,
            reasoner_tokens_used: 0,
            max_output_tokens_recovery_count: 0,
            has_attempted_prompt_too_long_compaction: false,
            last_transition: None,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                ordinal: 1,
                step: 3,
                kind: TranscriptUnitKind::CarryoverMessage,
                summary: "source reminder: /tmp/gabactl [tool|read]".to_string(),
                result_ref_id: None,
                primary_locator: Some("/tmp/gabactl".to_string()),
                evidence_refs: vec!["/tmp/gabactl".to_string()],
                working_sources: vec![WorkingSource {
                    kind: "tool".to_string(),
                    locator: "/tmp/gabactl".to_string(),
                    role: "validated".to_string(),
                    status: "read".to_string(),
                    why_it_matters: "validated external tool path".to_string(),
                    last_used_step: 3,
                    evidence_refs: vec!["/tmp/gabactl".to_string()],
                    page_reference: None,
                    extraction_method: None,
                    structured_summary: None,
                    preview_excerpt: Some("validated gabactl binary".to_string()),
                }],
                artifact_references: vec![ArtifactReference {
                    kind: "file".to_string(),
                    locator: "/tmp/result.json".to_string(),
                    status: "written".to_string(),
                }],
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            }]),
            reannounced_sources: vec![WorkingSource {
                kind: "tool".to_string(),
                locator: "/tmp/gabactl".to_string(),
                role: "validated".to_string(),
                status: "read".to_string(),
                why_it_matters: "validated external tool path".to_string(),
                last_used_step: 3,
                evidence_refs: vec!["/tmp/gabactl".to_string()],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: Some("validated gabactl binary".to_string()),
            }],
            reannounced_artifacts: vec![ArtifactReference {
                kind: "file".to_string(),
                locator: "/tmp/result.json".to_string(),
                status: "written".to_string(),
            }],
            ..ActiveContinuationWindow::default()
        }
    }

    #[test]
    fn reconstruct_follow_up_seed_uses_terminal_summary() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let continuation_window = sample_continuation("inspect gabactl");
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskCompleted,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "final_answer_summary": "validated gabactl and built path",
                "continuation_window": continuation_window
            }),
        };

        let seed = reconstruct_follow_up_seed_from_events(&[event], Some(&task_id))
            .expect("seed reconstruction should succeed")
            .expect("seed should exist");

        assert_eq!(seed.source_task_id, task_id);
        assert_eq!(seed.source_session_id, session_id);
        assert_eq!(seed.source_agent_id, agent_id);
        assert_eq!(seed.objective, "inspect gabactl");
        assert_eq!(
            seed.recent_context
                .as_ref()
                .and_then(|context| context.prior_answer_summary.as_deref()),
            Some("validated gabactl and built path")
        );
    }

    #[test]
    fn reconstruct_follow_up_seed_can_pair_blocked_terminal_with_earlier_continuation() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let continuation_window = sample_continuation("use the library to open chrome");
        let context_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: base_time,
            event_type: TimelineEventType::TaskContextAssembled,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": continuation_window
            }),
        };
        let blocked_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskBlocked,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "repeated the same step without new evidence"
            }),
        };

        let seed = reconstruct_follow_up_seed_from_events(&[context_event, blocked_event], None)
            .expect("seed reconstruction should succeed")
            .expect("seed should exist");

        assert_eq!(seed.objective, "use the library to open chrome");
        assert_eq!(
            seed.recent_context
                .as_ref()
                .and_then(|context| context.prior_answer_summary.as_deref()),
            Some("repeated the same step without new evidence")
        );
        assert_eq!(seed.continuation_window.reannounced_sources.len(), 1);
        assert_eq!(seed.continuation_window.reannounced_artifacts.len(), 1);
    }

    #[test]
    fn reconstruct_follow_up_seed_prefers_explicit_content_replacement_records() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let mut continuation_window = sample_continuation("inspect gabactl");
        continuation_window.content_replacements = ContentReplacementState {
            records: vec![ContentReplacementRecord {
                replacement_id: "result-1".to_string(),
                source_kind: "stored_result".to_string(),
                result_type: "run_command".to_string(),
                locator: Some("/tmp/gabactl".to_string()),
                persisted_path: Some("/tmp/result-1.json".to_string()),
                replacement_text: "[stored-result result-1] derived placeholder".to_string(),
            }],
        };
        let completed_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskCompleted,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "final_answer_summary": "validated gabactl path",
                "continuation_window": continuation_window
            }),
        };
        let replacement_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time,
            event_type: TimelineEventType::ContentReplacementsRecorded,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "records": [{
                    "replacement_id": "result-1",
                    "source_kind": "stored_result",
                    "result_type": "run_command",
                    "locator": "/tmp/gabactl",
                    "persisted_path": "/tmp/result-1.json",
                    "replacement_text": "[stored-result result-1] frozen exact replacement"
                }]
            }),
        };

        let seed = reconstruct_follow_up_seed_from_events(
            &[completed_event, replacement_event],
            Some(&task_id),
        )
        .expect("seed reconstruction should succeed")
        .expect("seed should exist");

        assert_eq!(
            seed.continuation_window.content_replacements.records[0].replacement_text,
            "[stored-result result-1] frozen exact replacement"
        );
    }

    #[test]
    fn reconstruct_latest_continuation_window_prefers_explicit_content_replacement_records() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let mut continuation_window = sample_continuation("inspect gabactl");
        continuation_window.content_replacements = ContentReplacementState {
            records: vec![ContentReplacementRecord {
                replacement_id: "result-1".to_string(),
                source_kind: "stored_result".to_string(),
                result_type: "run_command".to_string(),
                locator: Some("/tmp/gabactl".to_string()),
                persisted_path: Some("/tmp/result-1.json".to_string()),
                replacement_text: "[stored-result result-1] derived placeholder".to_string(),
            }],
        };
        let context_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskContextAssembled,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": continuation_window
            }),
        };
        let replacement_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: task_id.clone(),
            agent_id: AgentId::new(),
            timestamp: base_time,
            event_type: TimelineEventType::ContentReplacementsRecorded,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "records": [{
                    "replacement_id": "result-1",
                    "source_kind": "stored_result",
                    "result_type": "run_command",
                    "locator": "/tmp/gabactl",
                    "persisted_path": "/tmp/result-1.json",
                    "replacement_text": "[stored-result result-1] frozen exact replacement"
                }]
            }),
        };

        let (_, reconstructed) = reconstruct_latest_continuation_window_from_events(
            &[context_event, replacement_event],
            Some(&task_id),
        )
        .expect("continuation reconstruction should succeed")
        .expect("continuation should exist");

        assert_eq!(
            reconstructed.content_replacements.records[0].replacement_text,
            "[stored-result result-1] frozen exact replacement"
        );
    }

    #[test]
    fn reconstruct_latest_continuation_window_rebuilds_read_state_cache_from_action_results() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let mut continuation_window = sample_continuation("inspect startup.md");
        continuation_window.read_state_cache = Vec::new();
        let context_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskContextAssembled,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": continuation_window
            }),
        };
        let result_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time,
            event_type: TimelineEventType::ActionResultReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::FileRead {
                    path: "/tmp/startup.md".into(),
                    content: "alpha\nbeta\n".to_string(),
                    truncated: false,
                    start_line: 1,
                    line_count: 2,
                    total_lines: 2,
                    total_bytes: 11,
                    read_bytes: 11,
                }
            }),
        };

        let (_, reconstructed) = reconstruct_latest_continuation_window_from_events(
            &[context_event, result_event],
            Some(&task_id),
        )
        .expect("continuation reconstruction should succeed")
        .expect("continuation should exist");

        assert_eq!(reconstructed.read_state_cache.len(), 1);
        assert_eq!(
            reconstructed.read_state_cache[0].path,
            PathBuf::from("/tmp/startup.md")
        );
        assert!(!reconstructed.read_state_cache[0].was_partial);
    }

    #[test]
    fn reconstruct_latest_continuation_window_rebuilds_search_state_cache_from_action_results() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let mut continuation_window = sample_continuation("inspect swift sources");
        continuation_window.search_state_cache = Vec::new();
        let context_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskContextAssembled,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": continuation_window
            }),
        };
        let result_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time,
            event_type: TimelineEventType::ActionResultReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::TextSearch {
                    root: "/tmp/project".into(),
                    query: "struct AXActionCommand".to_string(),
                    output_mode: TextSearchOutputMode::FilesWithMatches,
                    matches: Vec::new(),
                    content: None,
                    filenames: vec![
                        "/tmp/project/Sources/gabactl/Commands/AX/AXActionCommand.swift".into()
                    ],
                    num_files: 1,
                    num_matches: 1,
                    truncated: true,
                    applied_offset: 50,
                    glob: Some("*.swift".to_string()),
                    case_insensitive: false,
                }
            }),
        };

        let (_, reconstructed) = reconstruct_latest_continuation_window_from_events(
            &[context_event, result_event],
            Some(&task_id),
        )
        .expect("continuation reconstruction should succeed")
        .expect("continuation should exist");

        assert_eq!(reconstructed.search_state_cache.len(), 1);
        assert_eq!(reconstructed.search_state_cache[0].kind, "search_text");
        assert_eq!(
            reconstructed.search_state_cache[0].root,
            PathBuf::from("/tmp/project")
        );
        assert_eq!(reconstructed.search_state_cache[0].applied_offset, 50);
        assert!(reconstructed.search_state_cache[0].truncated);
        assert_eq!(
            reconstructed.search_state_cache[0].samples,
            vec!["/tmp/project/Sources/gabactl/Commands/AX/AXActionCommand.swift".to_string()]
        );
    }

    #[test]
    fn recoverable_snapshot_prefers_explicit_content_replacement_records() {
        let task_id = TaskId::new();
        let session_id = SessionId::new();
        let agent_id = AgentId::new();
        let base_time = Utc::now();
        let mut continuation_window = sample_continuation("inspect gabactl");
        continuation_window.content_replacements = ContentReplacementState {
            records: vec![ContentReplacementRecord {
                replacement_id: "result-1".to_string(),
                source_kind: "stored_result".to_string(),
                result_type: "run_command".to_string(),
                locator: Some("/tmp/gabactl".to_string()),
                persisted_path: Some("/tmp/result-1.json".to_string()),
                replacement_text: "[stored-result result-1] derived placeholder".to_string(),
            }],
        };
        let blocked_event = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: base_time + Duration::seconds(1),
            event_type: TimelineEventType::TaskBlocked,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "provider transport failed",
                "continuation_window": continuation_window
            }),
        };
        let replacement_event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: task_id.clone(),
            agent_id: AgentId::new(),
            timestamp: base_time,
            event_type: TimelineEventType::ContentReplacementsRecorded,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "records": [{
                    "replacement_id": "result-1",
                    "source_kind": "stored_result",
                    "result_type": "run_command",
                    "locator": "/tmp/gabactl",
                    "persisted_path": "/tmp/result-1.json",
                    "replacement_text": "[stored-result result-1] frozen exact replacement"
                }]
            }),
        };

        let inspector = InspectController {
            memory: SqliteMemory::open_in_memory().expect("in-memory sqlite"),
        };
        inspector
            .append_timeline_event(&replacement_event)
            .expect("append replacement event");
        inspector
            .append_timeline_event(&blocked_event)
            .expect("append blocked event");

        let snapshot = inspector
            .recoverable_task_snapshot_by_id(&task_id.0)
            .expect("snapshot lookup should succeed")
            .expect("snapshot should exist");

        assert_eq!(
            snapshot.continuation_window.content_replacements.records[0].replacement_text,
            "[stored-result result-1] frozen exact replacement"
        );
    }

    #[test]
    fn summarize_terminal_tasks_only_counts_explicit_terminal_events() {
        let base = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskRecoveryContinued,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "blocked waiting for prompt_too_long_compaction"
            }),
        };
        let blocked = TimelineEvent {
            event_type: TimelineEventType::TaskBlocked,
            payload_json: json!({
                "reason": "repeated the same step without new evidence"
            }),
            ..base.clone()
        };

        let stats = summarize_terminal_tasks(&[base, blocked]);
        assert_eq!(stats.completed, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.cancelled, 0);
        assert_eq!(stats.blocked, 1);
    }

    #[test]
    fn summarize_recovery_stats_counts_explicit_recovery_reasons() {
        let base = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskRecoveryContinued,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "max_output_tokens_escalate"
            }),
        };
        let truncation = TimelineEvent {
            payload_json: json!({
                "reason": "max_output_tokens_recovery"
            }),
            ..base.clone()
        };
        let prompt_too_long = TimelineEvent {
            payload_json: json!({
                "reason": "prompt_too_long_compaction"
            }),
            ..base.clone()
        };
        let unknown = TimelineEvent {
            payload_json: json!({
                "reason": "other_transition"
            }),
            ..base.clone()
        };

        let stats = summarize_recovery_stats(&[base, truncation, prompt_too_long, unknown]);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.max_output_tokens_escalate, 1);
        assert_eq!(stats.max_output_tokens_recovery, 1);
        assert_eq!(stats.prompt_too_long_compaction, 1);
    }
}
