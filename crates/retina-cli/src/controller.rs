use crate::chat::StreamingMemory;
use crate::runtime::{
    normalize_root_manifest, open_memory, retina_home, root_agent_id, root_db_path, root_manifest,
};
use retina_kernel::Kernel;
use retina_llm_claude::{ClaudeReasoner, ClaudeRuntimeConfigSnapshot};
use retina_memory_sqlite::{MemoryStats, SqliteMemory};
use retina_shell_cli::{CliShell, ScopedShell};
use retina_traits::{Memory, Shell};
use retina_types::*;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
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
        let manifest = normalize_root_manifest(
            memory
                .load_manifest(&root_agent_id())?
                .unwrap_or(root_manifest()?),
        );
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
        let task = build_task_for_description(task_description.into())?;
        self.execute_with_root_state(task, move |kernel, task| {
            kernel.execute_task_with_config(task, config)
        })
    }

    pub fn spawn_task(
        &self,
        task_description: impl Into<String>,
        mut config: ExecutionConfig,
    ) -> RunningTask {
        let task_description = task_description.into();
        let task = build_task_for_description(task_description.clone())
            .unwrap_or_else(|_| Task::new(root_agent_id(), task_description));
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
            let outcome = run_task_catching_panics(&kernel, task_for_thread, config);
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

fn build_task_for_description(task_description: String) -> Result<Task> {
    let mut task = Task::new(root_agent_id(), task_description);
    task.recent_context = latest_recent_context_from_memory()?;
    Ok(task)
}

fn latest_recent_context_from_memory() -> Result<Option<RecentContext>> {
    let memory = open_memory(root_db_path()?)?;
    let events = memory.recent_states(200)?;
    Ok(latest_recent_context_from_events(&events))
}

fn latest_recent_context_from_events(events: &[TimelineEvent]) -> Option<RecentContext> {
    let completed = events
        .iter()
        .find(|event| matches!(event.event_type, TimelineEventType::TaskCompleted))?;
    let task_state = completed
        .payload_json
        .get("task_state")
        .and_then(|value| serde_json::from_value::<TaskState>(value.clone()).ok())?;

    let prior_answer_summary = completed
        .payload_json
        .get("final_answer_summary")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            task_state.recent_actions.iter().rev().find_map(|action| {
                action
                    .action
                    .strip_prefix("respond:")
                    .map(compact_answer_summary)
            })
        });

    Some(RecentContext {
        prior_objective: task_state.goal.objective.clone(),
        prior_answer_summary,
        sources: select_recent_sources(&task_state.working_sources),
        artifacts: select_recent_artifacts(&task_state.artifact_references),
    })
}

fn select_recent_sources(sources: &[WorkingSource]) -> Vec<WorkingSource> {
    let mut ranked = sources
        .iter()
        .filter(|source| source.kind != "command")
        .cloned()
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        source_rank(right)
            .cmp(&source_rank(left))
            .then_with(|| right.last_used_step.cmp(&left.last_used_step))
            .then_with(|| left.locator.cmp(&right.locator))
    });
    ranked.truncate(5);
    ranked
}

fn select_recent_artifacts(artifacts: &[ArtifactReference]) -> Vec<ArtifactReference> {
    let mut ranked = artifacts
        .iter()
        .filter(|artifact| artifact.kind != "command")
        .cloned()
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        artifact_rank(right)
            .cmp(&artifact_rank(left))
            .then_with(|| left.locator.cmp(&right.locator))
    });
    ranked.truncate(5);
    ranked
}

fn source_rank(source: &WorkingSource) -> u8 {
    let role_rank = match source.role.as_str() {
        "authoritative" => 4,
        "generated" => 3,
        "supporting" => 2,
        "candidate" => 1,
        _ => 0,
    };
    let status_rank = match source.status.as_str() {
        "read" | "excerpted" | "ingested" => 4,
        "created" | "written" | "overwritten" | "appended" | "command_changed" => 4,
        "matched_text" => 3,
        "matched" => 2,
        "listed" | "inspected" => 1,
        _ => 0,
    };
    role_rank * 10 + status_rank
}

fn artifact_rank(artifact: &ArtifactReference) -> u8 {
    match artifact.status.as_str() {
        "read" | "structured_read" | "extracted" => 5,
        "created" | "written" | "overwritten" | "appended" | "command_changed" => 4,
        "searched" => 3,
        "matched" => 2,
        "listed" | "inspected" => 1,
        _ => 0,
    }
}

fn compact_answer_summary(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(240).collect::<String>();
    if normalized.chars().count() > 240 {
        preview.push_str("...");
    }
    preview
}

impl RunningTask {
    pub fn try_recv(&self) -> std::result::Result<Result<Outcome>, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
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

    pub fn latest_task_state(&self) -> Result<Option<TaskState>> {
        let events = self.memory.recent_states(200)?;
        for event in events {
            if let Some(value) = event.payload_json.get("task_state") {
                let task_state = serde_json::from_value::<TaskState>(value.clone())
                    .map_err(|error| KernelError::Storage(error.to_string()))?;
                return Ok(Some(task_state));
            }
        }
        Ok(None)
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
        })
    }

    pub fn cleanup_memory(&self, config: ConsolidationConfig) -> Result<ConsolidationReport> {
        self.memory.consolidate(&config)
    }

    pub fn agent_registry(&self) -> Result<AgentRegistrySnapshot> {
        self.memory.agent_registry()
    }

    pub fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        self.memory.append_timeline_event(event)
    }
}

impl AgentController {
    fn execute_with_root_state(
        &self,
        task: Task,
        run: impl FnOnce(&Kernel, Task) -> Result<Outcome>,
    ) -> Result<Outcome> {
        update_root_manifest_state(
            AgentStatus::Running,
            AgentLifecyclePhase::Busy,
            Some("executing task"),
        )?;
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(&self.kernel, task)))
            .unwrap_or_else(|panic_payload| Err(task_panic_error(panic_payload)));
        let reset_result = update_root_manifest_state(
            AgentStatus::Idle,
            AgentLifecyclePhase::CoolingDown,
            Some("waiting for next task"),
        );
        match (outcome, reset_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(primary), Err(_secondary)) => Err(primary),
        }
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
            _ => {
                if matches!(event.event_type, TimelineEventType::TaskStepCompleted) {
                    continue;
                }
                if let TimelineEventType::TaskFailed = event.event_type {
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn completed_event(
        objective: &str,
        final_answer_summary: Option<&str>,
        mut task_state: TaskState,
        timestamp: chrono::DateTime<Utc>,
    ) -> TimelineEvent {
        task_state.goal.objective = objective.to_string();
        TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: root_agent_id(),
            timestamp,
            event_type: TimelineEventType::TaskCompleted,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "final_answer_summary": final_answer_summary,
                "task_state": task_state
            }),
        }
    }

    fn sample_task_state() -> TaskState {
        TaskState {
            goal: TaskGoal {
                objective: "placeholder".to_string(),
                success_criteria: vec![],
                constraints: vec![],
            },
            intent_hint: None,
            reasoner_framing: None,
            progress: TaskProgress {
                current_phase: "working".to_string(),
                current_step: 2,
                max_steps: 50,
                completed_checkpoints: vec![],
                verified_facts: vec![],
                output_written: false,
                output_verified: false,
                remaining_obligation: None,
                pending_deliverable: None,
                target_output_path: None,
                target_output_exists: false,
            },
            frontier: TaskFrontier::default(),
            recent_actions: vec![RecentActionSummary {
                step: 2,
                action: "respond:summary".to_string(),
                outcome: "responded to operator".to_string(),
                artifact_refs: vec![],
            }],
            working_sources: vec![
                WorkingSource {
                    kind: "command".to_string(),
                    locator: "dir".to_string(),
                    role: "supporting".to_string(),
                    status: "executed".to_string(),
                    why_it_matters: "noise".to_string(),
                    last_used_step: 1,
                    evidence_refs: vec![],
                    page_reference: None,
                    extraction_method: Some("run_command".to_string()),
                    structured_summary: None,
                    preview_excerpt: None,
                },
                WorkingSource {
                    kind: "file".to_string(),
                    locator: "C:/texts/Watcher.txt".to_string(),
                    role: "authoritative".to_string(),
                    status: "read".to_string(),
                    why_it_matters: "real source".to_string(),
                    last_used_step: 2,
                    evidence_refs: vec![],
                    page_reference: None,
                    extraction_method: Some("text_read".to_string()),
                    structured_summary: None,
                    preview_excerpt: Some("watcher notes".to_string()),
                },
            ],
            artifact_references: vec![
                ArtifactReference {
                    kind: "command".to_string(),
                    locator: "dir".to_string(),
                    status: "executed".to_string(),
                },
                ArtifactReference {
                    kind: "file".to_string(),
                    locator: "C:/texts/Watcher.txt".to_string(),
                    status: "read".to_string(),
                },
            ],
            avoid: vec![],
            compaction: None,
        }
    }

    #[test]
    fn latest_recent_context_uses_only_latest_completed_task() {
        let older = completed_event(
            "list files in texts",
            Some("older"),
            sample_task_state(),
            Utc::now() - chrono::Duration::minutes(2),
        );
        let newer = completed_event(
            "what is Watcher.txt about?",
            Some("newer"),
            sample_task_state(),
            Utc::now(),
        );

        let recent = latest_recent_context_from_events(&[newer.clone(), older]).unwrap();
        assert_eq!(recent.prior_objective, "what is Watcher.txt about?");
        assert_eq!(recent.prior_answer_summary.as_deref(), Some("newer"));
    }

    #[test]
    fn latest_recent_context_prefers_authoritative_sources_and_non_command_artifacts() {
        let recent = latest_recent_context_from_events(&[completed_event(
            "list files in texts",
            None,
            sample_task_state(),
            Utc::now(),
        )])
        .unwrap();

        assert_eq!(recent.sources.len(), 1);
        assert_eq!(recent.sources[0].locator, "C:/texts/Watcher.txt");
        assert_eq!(recent.artifacts.len(), 1);
        assert_eq!(recent.artifacts[0].locator, "C:/texts/Watcher.txt");
        assert_eq!(
            recent.prior_answer_summary.as_deref(),
            Some("summary")
        );
    }
}
