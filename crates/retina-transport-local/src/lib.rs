mod specialists;

use chrono::Utc;
use retina_kernel::Kernel;
use retina_llm_claude::ClaudeReasoner;
use retina_mcp_client::{ConfiguredMcpRuntime, default_config_path};
use retina_memory_sqlite::{SqliteMemory, write_manifest};
use retina_runtime::{RuntimeTaskKind, RuntimeTaskStatus, TaskSupervisor, outcome_summary};
use retina_shell_cli::{CliShell, ScopedShell};
use retina_tools::ToolPolicy;
use retina_traits::{AgentRuntime, McpRuntime, Memory};
use retina_types::*;
use serde_json::Value;
use specialists::{apply_definition, resolve_definition, scoped_authority};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LocalTransportConfig {
    pub db_path: PathBuf,
    pub agents_dir: PathBuf,
    pub retina_home: PathBuf,
}

#[derive(Clone)]
pub struct LocalAgentRuntimeService {
    supervisor: TaskSupervisor,
    config: LocalTransportConfig,
    parent_authority: AgentAuthority,
}

impl LocalAgentRuntimeService {
    pub fn new(
        supervisor: TaskSupervisor,
        config: LocalTransportConfig,
        parent_authority: AgentAuthority,
    ) -> Self {
        Self {
            supervisor,
            config,
            parent_authority,
        }
    }

    fn open_memory(&self) -> Result<SqliteMemory> {
        SqliteMemory::open(&self.config.db_path)
    }

    fn manifest_path_for(&self, agent_id: &AgentId) -> PathBuf {
        self.config
            .agents_dir
            .join(&agent_id.0)
            .join("manifest.toml")
    }

    fn persist_manifest(&self, manifest: &AgentManifest) -> Result<()> {
        let memory = self.open_memory()?;
        memory.save_manifest(manifest)?;
        write_manifest(self.manifest_path_for(&manifest.agent_id), manifest)
    }

    fn update_manifest_state(
        &self,
        agent_id: &AgentId,
        status: AgentStatus,
        phase: AgentLifecyclePhase,
        reason: Option<&str>,
    ) -> Result<()> {
        let memory = self.open_memory()?;
        if let Some(manifest) = memory.update_manifest_lifecycle(agent_id, status, phase, reason)? {
            write_manifest(self.manifest_path_for(agent_id), &manifest)?;
        }
        Ok(())
    }

    fn load_manifest(&self, agent_id: &AgentId) -> Result<Option<AgentManifest>> {
        self.open_memory()?.load_manifest(agent_id)
    }

    fn route_existing_manifest(
        &self,
        _request: &RouteAgentRequest,
        agent_id: &AgentId,
    ) -> Result<AgentManifest> {
        let manifest = self.load_manifest(agent_id)?.ok_or_else(|| {
            KernelError::Execution(format!("no manifest found for routed agent {}", agent_id.0))
        })?;
        self.refresh_specialist_manifest(manifest)
    }

    fn current_mcp_snapshot(&self) -> Result<McpRegistrySnapshot> {
        ConfiguredMcpRuntime::new(default_config_path(&self.config.retina_home)).snapshot()
    }

    fn ensure_mcp_requirements(&self, manifest: &AgentManifest) -> Result<()> {
        let missing = missing_required_mcp_servers(
            &manifest.required_mcp_servers,
            &self.current_mcp_snapshot()?,
        );
        if missing.is_empty() {
            return Ok(());
        }
        Err(KernelError::Execution(format!(
            "agent {} requires unavailable MCP server(s): {}",
            manifest.agent_id.0,
            missing.join(", ")
        )))
    }

    fn refresh_specialist_manifest(&self, manifest: AgentManifest) -> Result<AgentManifest> {
        let Some(domain) = specialist_domain_from_agent_id(&manifest.agent_id) else {
            return Ok(manifest);
        };
        let definition = resolve_definition(&self.config.agents_dir, &domain)?;
        let refreshed = apply_definition(manifest, &definition, &self.parent_authority);
        self.persist_manifest(&refreshed)?;
        Ok(refreshed)
    }

    fn spawn_specialist_manifest(
        &self,
        request: &RouteAgentRequest,
        domain: &str,
        capability: &str,
    ) -> Result<AgentManifest> {
        let agent_id = specialist_agent_id(domain);
        if let Some(existing) = self.load_manifest(&agent_id)? {
            return self.refresh_specialist_manifest(existing);
        }

        let now = Utc::now();
        let definition = resolve_definition(&self.config.agents_dir, domain)?;
        let manifest = apply_definition(
            AgentManifest {
                agent_id: agent_id.clone(),
                domain: domain.to_string(),
                status: AgentStatus::Idle,
                description: format!("Local {domain} specialist for {capability}."),
                role_prompt: None,
                initial_prompt: None,
                model_id: None,
                created_at: now,
                updated_at: now,
                parent_agent_id: Some(request.parent_task.agent_id.clone()),
                capabilities: specialist_capability_summary(domain, capability),
                allowed_tools: Vec::new(),
                denied_tools: Vec::new(),
                required_mcp_servers: Vec::new(),
                authority: self.parent_authority.clone(),
                lifecycle: AgentLifecycle::ready(),
                budget: AgentBudget::default(),
            },
            &definition,
            &self.parent_authority,
        );
        self.persist_manifest(&manifest)?;
        Ok(manifest)
    }

    fn run_manifest_task(
        &self,
        manifest: AgentManifest,
        parent_task: &Task,
        parent_continuation_window: Option<ActiveContinuationWindow>,
        control: Option<&ExecutionControlHandle>,
        route_label: &str,
    ) -> Result<DelegatedTaskResult> {
        self.update_manifest_state(
            &manifest.agent_id,
            AgentStatus::Running,
            AgentLifecyclePhase::Busy,
            Some("executing routed task"),
        )?;

        let runtime_task = build_specialist_task(
            parent_task,
            parent_continuation_window,
            &manifest,
            route_label,
        );
        let runtime_task_id = runtime_task.id.clone();
        let runtime_service = Arc::new(Self::new(
            self.supervisor.clone(),
            self.config.clone(),
            manifest.authority.clone(),
        ));
        let mcp_runtime = Arc::new(ConfiguredMcpRuntime::new(default_config_path(
            &self.config.retina_home,
        )));
        let child_control = control
            .cloned()
            .unwrap_or_else(|| ExecutionControl::new().handle());
        let child_control_for_kernel = child_control.clone();
        let child_task_for_thread = runtime_task.clone();
        let child_manifest = manifest.clone();
        let config = self.config.clone();
        let supervisor = self.supervisor.clone();
        let runtime_task_id_for_thread = runtime_task_id.clone();
        let handle = self.supervisor.spawn(
            runtime_task,
            RuntimeTaskKind::Specialist,
            child_control,
            move || {
                let memory = SqliteMemory::open(&config.db_path)?;
                let registry = memory.agent_registry()?;
                let child_kernel = Kernel::new_with_runtime(
                    Box::new(ScopedShell::new(
                        CliShell::new(),
                        child_manifest.authority.clone(),
                    )),
                    Box::new(specialist_reasoner(&child_manifest)),
                    Box::new(memory),
                    registry,
                    ToolPolicy::from_authority(&child_manifest.authority),
                    Some(runtime_service),
                    Some(mcp_runtime),
                )?;
                let outcome = child_kernel.execute_task_with_config(
                    child_task_for_thread,
                    ExecutionConfig {
                        max_steps: child_manifest.budget.max_steps_per_task,
                        control: Some(child_control_for_kernel),
                    },
                );
                let state_reason = outcome_summary(&outcome);
                if let Ok(memory) = SqliteMemory::open(&config.db_path) {
                    if let Ok(Some(updated)) = memory.update_manifest_lifecycle(
                        &child_manifest.agent_id,
                        AgentStatus::Idle,
                        AgentLifecyclePhase::CoolingDown,
                        Some(&state_reason),
                    ) {
                        let _ = write_manifest(
                            config
                                .agents_dir
                                .join(&child_manifest.agent_id.0)
                                .join("manifest.toml"),
                            &updated,
                        );
                    }
                }
                let _ = supervisor.registry().snapshot(&runtime_task_id_for_thread);
                outcome
            },
        );
        let outcome = handle.recv()?;
        let snapshot = self.supervisor.registry().snapshot(&runtime_task_id);
        let status = snapshot
            .as_ref()
            .map(|task| delegated_status(task.status.clone()))
            .unwrap_or_else(|| delegated_status_from_outcome(&outcome));
        let summary = snapshot
            .as_ref()
            .and_then(|task| task.progress_summary.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| outcome_summary(&Ok(outcome.clone())));
        let transcript_excerpt = delegated_task_transcript(&self.config.db_path, &runtime_task_id);
        append_transcript_to_output(
            snapshot.as_ref().and_then(|task| task.output_path.as_ref()),
            transcript_excerpt.as_deref(),
        );
        Ok(DelegatedTaskResult {
            agent_id: manifest.agent_id,
            task_id: runtime_task_id,
            parent_task_id: Some(parent_task.id.clone()),
            status,
            summary,
            transcript_excerpt,
            output_path: snapshot.and_then(|task| task.output_path),
        })
    }
}

impl AgentRuntime for LocalAgentRuntimeService {
    fn spawn_local_agent(
        &self,
        request: &SpawnAgentRequest,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<DelegatedTaskResult> {
        let child_agent_id = AgentId(format!("local-{}", TaskId::new().0));
        let child_authority = child_authority(
            &self.parent_authority,
            &request.allowed_tools,
            &request.denied_tools,
        );
        let child_task = build_child_task(request, child_agent_id.clone());
        let child_task_id = child_task.id.clone();
        let child_control = control
            .cloned()
            .unwrap_or_else(|| ExecutionControl::new().handle());
        let child_control_for_kernel = child_control.clone();
        let child_task_for_thread = child_task.clone();
        let child_authority_for_kernel = child_authority.clone();
        let config = self.config.clone();
        let supervisor = self.supervisor.clone();
        let mcp_runtime = Arc::new(ConfiguredMcpRuntime::new(default_config_path(
            &self.config.retina_home,
        )));
        let child_task_id_for_thread = child_task_id.clone();
        let handle = self.supervisor.spawn(
            child_task,
            RuntimeTaskKind::LocalAgent,
            child_control,
            move || {
                let memory = SqliteMemory::open(&config.db_path)?;
                let registry = memory.agent_registry()?;
                let child_kernel = Kernel::new_with_runtime(
                    Box::new(ScopedShell::new(
                        CliShell::new(),
                        child_authority_for_kernel.clone(),
                    )),
                    Box::new(ClaudeReasoner::new()),
                    Box::new(memory),
                    registry,
                    ToolPolicy::from_authority(&child_authority_for_kernel),
                    None,
                    Some(mcp_runtime),
                )?;
                let outcome = child_kernel.execute_task_with_config(
                    child_task_for_thread,
                    ExecutionConfig {
                        max_steps: 6,
                        control: Some(child_control_for_kernel),
                    },
                );
                let _ = supervisor.registry().snapshot(&child_task_id_for_thread);
                outcome
            },
        );
        let outcome = handle.recv()?;
        let snapshot = self.supervisor.registry().snapshot(&child_task_id);
        let status = snapshot
            .as_ref()
            .map(|task| delegated_status(task.status.clone()))
            .unwrap_or_else(|| delegated_status_from_outcome(&outcome));
        let summary = snapshot
            .as_ref()
            .and_then(|task| task.progress_summary.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| outcome_summary(&Ok(outcome.clone())));
        let transcript_excerpt = delegated_task_transcript(&self.config.db_path, &child_task_id);
        append_transcript_to_output(
            snapshot.as_ref().and_then(|task| task.output_path.as_ref()),
            transcript_excerpt.as_deref(),
        );
        Ok(DelegatedTaskResult {
            agent_id: child_agent_id,
            task_id: child_task_id,
            parent_task_id: Some(request.parent_task.id.clone()),
            status,
            summary,
            transcript_excerpt,
            output_path: snapshot.and_then(|task| task.output_path),
        })
    }

    fn execute_routing_decision(
        &self,
        request: &RouteAgentRequest,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<DelegatedTaskResult> {
        match &request.decision {
            RoutingDecision::HandleDirectly => Err(KernelError::Execution(
                "local transport received HandleDirectly routing decision".to_string(),
            )),
            RoutingDecision::RouteToExisting(agent_id) => {
                let manifest = self.route_existing_manifest(request, agent_id)?;
                self.ensure_mcp_requirements(&manifest)?;
                self.run_manifest_task(
                    manifest,
                    &request.parent_task,
                    request.parent_continuation_window.clone(),
                    control,
                    "route_existing",
                )
            }
            RoutingDecision::Reactivate(agent_id) => {
                let manifest = self.route_existing_manifest(request, agent_id)?;
                self.ensure_mcp_requirements(&manifest)?;
                self.update_manifest_state(
                    agent_id,
                    AgentStatus::Idle,
                    AgentLifecyclePhase::Ready,
                    Some("reactivated for new task"),
                )?;
                self.run_manifest_task(
                    manifest,
                    &request.parent_task,
                    request.parent_continuation_window.clone(),
                    control,
                    "reactivate",
                )
            }
            RoutingDecision::SpawnSpecialist { domain, capability } => {
                let manifest = self.spawn_specialist_manifest(request, domain, capability)?;
                self.ensure_mcp_requirements(&manifest)?;
                self.run_manifest_task(
                    manifest,
                    &request.parent_task,
                    request.parent_continuation_window.clone(),
                    control,
                    "spawn_specialist",
                )
            }
        }
    }
}

fn build_child_task(request: &SpawnAgentRequest, child_agent_id: AgentId) -> Task {
    let mut task = Task::spawn_child(
        &request.parent_task,
        child_agent_id,
        format!(
            "Delegated child task.\nParent objective: {}\nSubtask: {}\nReturn a grounded result for the parent worker.",
            request.parent_task.description, request.prompt
        ),
        request.parent_task.recent_context.clone(),
    );
    if let Some(continuation_window) = request.parent_continuation_window.clone() {
        task.resume_context = Some(TaskResumeContext {
            source_task_id: request.parent_task.id.clone(),
            source_session_id: request.parent_task.session_id.clone(),
            objective: request.parent_task.description.clone(),
            continuation_window,
            resume_reason: "delegated child sidechain".to_string(),
        });
    }
    if !request.allowed_tools.is_empty() {
        task.metadata
            .insert("allowed_tools".to_string(), request.allowed_tools.join(","));
    }
    let mut denied_tools = request.denied_tools.clone();
    if !denied_tools.iter().any(|tool| tool == "agent_spawn") {
        denied_tools.push("agent_spawn".to_string());
    }
    task.metadata
        .insert("denied_tools".to_string(), denied_tools.join(","));
    task.metadata.insert(
        "delegated_from_agent".to_string(),
        request.parent_task.agent_id.0.clone(),
    );
    task
}

fn build_specialist_task(
    parent_task: &Task,
    parent_continuation_window: Option<ActiveContinuationWindow>,
    manifest: &AgentManifest,
    route_label: &str,
) -> Task {
    let child_description = if let Some(initial_prompt) = manifest.initial_prompt.as_deref() {
        if initial_prompt.trim().is_empty() {
            format!(
                "Delegated specialist task.\nParent objective: {}\nReturn a grounded result for the parent worker.",
                parent_task.description
            )
        } else {
            format!(
                "Delegated specialist task.\nParent objective: {}\nSpecialist directive: {}\nReturn a grounded result for the parent worker.",
                parent_task.description,
                initial_prompt.trim()
            )
        }
    } else {
        format!(
            "Delegated specialist task.\nParent objective: {}\nReturn a grounded result for the parent worker.",
            parent_task.description
        )
    };
    let mut task = Task::spawn_child(
        parent_task,
        manifest.agent_id.clone(),
        child_description,
        parent_task.recent_context.clone(),
    );
    if let Some(continuation_window) = parent_continuation_window {
        task.resume_context = Some(TaskResumeContext {
            source_task_id: parent_task.id.clone(),
            source_session_id: parent_task.session_id.clone(),
            objective: parent_task.description.clone(),
            continuation_window,
            resume_reason: "delegated specialist sidechain".to_string(),
        });
    }
    task.metadata.insert(
        "delegated_from_agent".to_string(),
        parent_task.agent_id.0.clone(),
    );
    task.metadata
        .insert("routing_origin".to_string(), route_label.to_string());
    if let Some(role_prompt) = manifest.role_prompt.as_deref() {
        if !role_prompt.trim().is_empty() {
            task.metadata
                .insert("agent_role_prompt".to_string(), role_prompt.to_string());
        }
    }
    if let Some(initial_prompt) = manifest.initial_prompt.as_deref() {
        if !initial_prompt.trim().is_empty() {
            task.metadata.insert(
                "agent_initial_prompt".to_string(),
                initial_prompt.to_string(),
            );
        }
    }
    if !manifest.allowed_tools.is_empty() {
        task.metadata.insert(
            "allowed_tools".to_string(),
            manifest.allowed_tools.join(","),
        );
    }
    if !manifest.denied_tools.is_empty() {
        task.metadata
            .insert("denied_tools".to_string(), manifest.denied_tools.join(","));
    }
    if !manifest.required_mcp_servers.is_empty() {
        task.metadata.insert(
            "required_mcp_servers".to_string(),
            manifest.required_mcp_servers.join(","),
        );
    }
    task.metadata.insert(
        "max_reasoner_calls_per_task".to_string(),
        manifest.budget.max_reasoner_calls_per_task.to_string(),
    );
    task.metadata.insert(
        "max_tokens_per_task".to_string(),
        manifest.budget.max_tokens_per_task.to_string(),
    );
    task
}

fn child_authority(
    parent: &AgentAuthority,
    allowed_tools: &[String],
    denied_tools: &[String],
) -> AgentAuthority {
    scoped_authority(parent, allowed_tools, denied_tools, false)
}

fn specialist_reasoner(manifest: &AgentManifest) -> ClaudeReasoner {
    manifest
        .model_id
        .as_ref()
        .map(|model_id| ClaudeReasoner::with_model(model_id.clone()))
        .unwrap_or_default()
}

fn specialist_agent_id(domain: &str) -> AgentId {
    AgentId(format!("specialist-{}", slug(domain)))
}

fn specialist_domain_from_agent_id(agent_id: &AgentId) -> Option<String> {
    agent_id
        .0
        .strip_prefix("specialist-")
        .map(|value| value.to_string())
}

fn specialist_capability_summary(domain: &str, capability: &str) -> Vec<String> {
    let mut caps = vec![domain.to_string()];
    for token in capability
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(|token| token.to_lowercase())
    {
        if !caps.iter().any(|existing| existing == &token) {
            caps.push(token);
        }
    }
    caps
}

fn delegated_task_transcript(db_path: &std::path::Path, task_id: &TaskId) -> Option<String> {
    let memory = SqliteMemory::open(db_path).ok()?;
    let mut events = memory.recent_states(256).ok()?;
    events.retain(|event| event.task_id == *task_id);
    events.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    let mut lines = Vec::new();
    for event in events {
        match event.event_type {
            TimelineEventType::ActionDispatched => {
                if let Some(action) = event
                    .payload_json
                    .get("action")
                    .and_then(|value| value.as_str())
                {
                    lines.push(format!("action: {}", compact_action_trace(action)));
                }
            }
            TimelineEventType::ActionResultReceived => {
                if let Some(line) = summarize_action_result(&event.payload_json) {
                    lines.push(line);
                }
            }
            TimelineEventType::TaskFailed | TimelineEventType::TaskBlocked => {
                if let Some(reason) = event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                {
                    lines.push(format!("blocked: {reason}"));
                }
            }
            TimelineEventType::TaskCompleted => {
                if let Some(summary) = event
                    .payload_json
                    .get("final_answer_summary")
                    .and_then(|value| value.as_str())
                {
                    lines.push(format!("done: {summary}"));
                }
            }
            _ => {}
        }
    }

    if lines.is_empty() {
        None
    } else {
        let start = lines.len().saturating_sub(8);
        Some(lines.into_iter().skip(start).collect::<Vec<_>>().join("\n"))
    }
}

fn summarize_action_result(payload_json: &Value) -> Option<String> {
    let result = payload_json.get("result")?;
    if let Some(directory) = result.get("DirectoryListing") {
        let root = directory.get("root").and_then(|value| value.as_str())?;
        let summary = directory.get("summary");
        let total = summary
            .and_then(|value| value.get("total_entries"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                directory
                    .get("entries")
                    .and_then(|value| value.as_array())
                    .map(|entries| entries.len() as u64)
            })
            .unwrap_or_default();
        let file_count = summary
            .and_then(|value| value.get("file_count"))
            .and_then(|value| value.as_u64())
            .unwrap_or_default();
        let dir_count = summary
            .and_then(|value| value.get("dir_count"))
            .and_then(|value| value.as_u64())
            .unwrap_or_default();
        return Some(format!(
            "observed: listed {root} ({total} entries, {file_count} files, {dir_count} dirs)"
        ));
    }
    if let Some(file) = result.get("FileRead") {
        let path = file.get("path").and_then(|value| value.as_str())?;
        return Some(format!("observed: read file {path}"));
    }
    if let Some(doc) = result.get("DocumentText") {
        let path = doc.get("path").and_then(|value| value.as_str())?;
        return Some(format!("observed: extracted document text {path}"));
    }
    if let Some(command) = result.get("Command") {
        let raw = command
            .get("stdout")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim();
        let preview = compact_transcript_text(raw, 80);
        return Some(if preview.is_empty() {
            "observed: command completed".to_string()
        } else {
            format!("observed: command output {preview}")
        });
    }
    if let Some(inspection) = result.get("Inspection") {
        let count = inspection
            .get("files")
            .and_then(|value| value.as_array())
            .map(|entries| entries.len())
            .unwrap_or_default();
        return Some(format!("observed: inspected path ({count} item(s))"));
    }
    if let Some(response) = result.get("Response") {
        let message = response.get("message").and_then(|value| value.as_str())?;
        return Some(format!(
            "observed: response {}",
            compact_transcript_text(message, 180)
        ));
    }
    None
}

fn compact_transcript_text(value: &str, max_chars: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut preview = collapsed.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    preview
}

fn append_transcript_to_output(output_path: Option<&PathBuf>, transcript: Option<&str>) {
    let Some(path) = output_path else {
        return;
    };
    let Some(transcript) = transcript else {
        return;
    };
    if transcript.trim().is_empty() {
        return;
    }
    let Ok(existing) = std::fs::read_to_string(path) else {
        return;
    };
    if existing.contains(transcript) {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
        let _ = writeln!(file, "\nchild_trace:\n{transcript}");
    }
}

fn compact_action_trace(action: &str) -> String {
    if let Some(message) = action.strip_prefix("respond:") {
        return format!("respond:{}", compact_transcript_text(message, 180));
    }
    compact_transcript_text(action, 180)
}

fn slug(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn delegated_status(status: RuntimeTaskStatus) -> DelegatedTaskStatus {
    match status {
        RuntimeTaskStatus::Completed => DelegatedTaskStatus::Completed,
        RuntimeTaskStatus::Failed => DelegatedTaskStatus::Failed,
        RuntimeTaskStatus::Blocked => DelegatedTaskStatus::Blocked,
        RuntimeTaskStatus::Killed => DelegatedTaskStatus::Killed,
        RuntimeTaskStatus::Pending | RuntimeTaskStatus::Running => DelegatedTaskStatus::Blocked,
    }
}

fn delegated_status_from_outcome(outcome: &Outcome) -> DelegatedTaskStatus {
    match outcome {
        Outcome::Success(_) => DelegatedTaskStatus::Completed,
        Outcome::Failure(_) => DelegatedTaskStatus::Failed,
        Outcome::Blocked(reason) if reason.contains("cancelled") => DelegatedTaskStatus::Killed,
        Outcome::Blocked(_) => DelegatedTaskStatus::Blocked,
    }
}

fn missing_required_mcp_servers(
    required: &[String],
    snapshot: &McpRegistrySnapshot,
) -> Vec<String> {
    if required.is_empty() {
        return Vec::new();
    }
    required
        .iter()
        .filter(|required_name| {
            !snapshot.servers.iter().any(|server| {
                server.error.is_none() && server.name.eq_ignore_ascii_case(required_name)
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_memory_sqlite::SqliteMemory;
    use retina_runtime::TaskSupervisor;
    use tempfile::tempdir;

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
    }

    fn sample_parent_continuation(objective: &str) -> ActiveContinuationWindow {
        ActiveContinuationWindow {
            objective: objective.to_string(),
            current_step: 4,
            max_steps: 12,
            reasoner_tokens_used: 0,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            reannounced_sources: vec![WorkingSource {
                kind: "tool".to_string(),
                locator: "/tmp/gabactl".to_string(),
                role: "validated".to_string(),
                status: "read".to_string(),
                why_it_matters: "validated tool path".to_string(),
                last_used_step: 4,
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

    fn test_service() -> (tempfile::TempDir, LocalAgentRuntimeService) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("agent.db");
        let agents_dir = dir.path().join("agents");
        let runtime_dir = dir.path().join("runtime");
        let retina_home = dir.path().to_path_buf();
        let memory = must(SqliteMemory::open(&db_path));
        let supervisor = TaskSupervisor::new(runtime_dir).with_store(Arc::new(memory));
        (
            dir,
            LocalAgentRuntimeService::new(
                supervisor,
                LocalTransportConfig {
                    db_path,
                    agents_dir,
                    retina_home,
                },
                AgentAuthority::default(),
            ),
        )
    }

    #[test]
    fn spawned_specialist_manifest_persists_tool_scope() {
        let (_dir, service) = test_service();
        let parent_task = Task::new(AgentId("root".to_string()), "research startup.md");
        let manifest = must(service.spawn_specialist_manifest(
            &RouteAgentRequest {
                parent_task,
                parent_continuation_window: None,
                decision: RoutingDecision::SpawnSpecialist {
                    domain: "research".to_string(),
                    capability: "read and summarize documents".to_string(),
                },
            },
            "research",
            "read and summarize documents",
        ));

        assert_eq!(manifest.allowed_tools, vec!["*".to_string()]);
        assert!(manifest.denied_tools.is_empty());
        assert!(manifest.authority.allow_command_execution);
    }

    #[test]
    fn spawned_specialist_manifest_uses_custom_definition_file_when_present() {
        let (dir, service) = test_service();
        let definition_path = dir
            .path()
            .join("agents")
            .join("specialist-research")
            .join("definition.toml");
        std::fs::create_dir_all(
            definition_path
                .parent()
                .unwrap_or_else(|| panic!("definition path missing parent")),
        )
        .unwrap_or_else(|error| panic!("mkdir: {error}"));
        std::fs::write(
            &definition_path,
            r#"description = "Custom research specialist"
role_prompt = "You are a custom research specialist."
initial_prompt = "Return only the delegated answer."
model_id = "claude-sonnet-4-20250514"
capabilities = ["research", "custom"]
allowed_tools = ["read_file", "respond"]
denied_tools = ["run_command"]
required_mcp_servers = ["docs"]
max_steps = 9
"#,
        )
        .unwrap_or_else(|error| panic!("write definition: {error}"));

        let manifest = must(service.spawn_specialist_manifest(
            &RouteAgentRequest {
                parent_task: Task::new(AgentId("root".to_string()), "research startup.md"),
                parent_continuation_window: None,
                decision: RoutingDecision::SpawnSpecialist {
                    domain: "research".to_string(),
                    capability: "read and summarize documents".to_string(),
                },
            },
            "research",
            "read and summarize documents",
        ));

        assert_eq!(manifest.description, "Custom research specialist");
        assert_eq!(
            manifest.role_prompt.as_deref(),
            Some("You are a custom research specialist.")
        );
        assert_eq!(
            manifest.initial_prompt.as_deref(),
            Some("Return only the delegated answer.")
        );
        assert_eq!(
            manifest.model_id.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
        assert_eq!(manifest.allowed_tools, vec!["read_file", "respond"]);
        assert_eq!(manifest.required_mcp_servers, vec!["docs"]);
        assert_eq!(manifest.budget.max_steps_per_task, 9);
    }

    #[test]
    fn specialist_task_carries_manifest_tool_metadata() {
        let now = Utc::now();
        let manifest = AgentManifest {
            agent_id: AgentId("specialist-research".to_string()),
            domain: "research".to_string(),
            status: AgentStatus::Idle,
            description: "research specialist".to_string(),
            role_prompt: Some("You are a local research specialist.".to_string()),
            initial_prompt: Some("Return a concise synthesis.".to_string()),
            model_id: Some("claude-sonnet-4-20250514".to_string()),
            created_at: now,
            updated_at: now,
            parent_agent_id: Some(AgentId("root".to_string())),
            capabilities: vec!["research".to_string()],
            allowed_tools: vec!["read_file".to_string(), "search_text".to_string()],
            denied_tools: vec!["run_command".to_string()],
            required_mcp_servers: vec!["docs".to_string()],
            authority: AgentAuthority::default(),
            lifecycle: AgentLifecycle::ready(),
            budget: AgentBudget::default(),
        };
        let task = build_specialist_task(
            &Task::new(AgentId("root".to_string()), "research startup.md"),
            None,
            &manifest,
            "spawn_specialist",
        );

        assert!(task.description.contains("Delegated specialist task."));
        assert!(
            task.description
                .contains("Parent objective: research startup.md")
        );
        assert!(
            task.description
                .contains("Specialist directive: Return a concise synthesis.")
        );
        assert!(
            task.description
                .contains("Return a grounded result for the parent worker.")
        );

        assert_eq!(
            task.metadata.get("allowed_tools").map(String::as_str),
            Some("read_file,search_text")
        );
        assert_eq!(
            task.metadata.get("denied_tools").map(String::as_str),
            Some("run_command")
        );
        assert_eq!(
            task.metadata
                .get("required_mcp_servers")
                .map(String::as_str),
            Some("docs")
        );
        assert_eq!(
            task.metadata.get("agent_role_prompt").map(String::as_str),
            Some("You are a local research specialist.")
        );
        assert_eq!(
            task.metadata
                .get("agent_initial_prompt")
                .map(String::as_str),
            Some("Return a concise synthesis.")
        );
        assert_eq!(
            task.metadata
                .get("max_reasoner_calls_per_task")
                .map(String::as_str),
            Some("8")
        );
        assert_eq!(
            task.metadata.get("max_tokens_per_task").map(String::as_str),
            Some("8192")
        );
    }

    #[test]
    fn build_child_task_inherits_parent_continuation_as_resume_context() {
        let parent_task = Task::new(AgentId("root".to_string()), "inspect gabactl");
        let parent_task_id = parent_task.id.clone();
        let parent_session_id = parent_task.session_id.clone();
        let continuation = sample_parent_continuation(&parent_task.description);
        let task = build_child_task(
            &SpawnAgentRequest {
                parent_task: parent_task.clone(),
                parent_continuation_window: Some(continuation.clone()),
                prompt: "use the validated library to open chrome".to_string(),
                allowed_tools: vec!["run_command".to_string()],
                denied_tools: Vec::new(),
            },
            AgentId("local-child".to_string()),
        );

        let resume = task.resume_context.expect("child resume context");
        assert_eq!(resume.source_task_id, parent_task_id);
        assert_eq!(resume.source_session_id, parent_session_id);
        assert_eq!(resume.objective, "inspect gabactl");
        assert_eq!(resume.resume_reason, "delegated child sidechain");
        assert_eq!(resume.continuation_window.objective, continuation.objective);
        assert_eq!(resume.continuation_window.reannounced_sources.len(), 1);
        assert_eq!(
            task.metadata.get("denied_tools").map(String::as_str),
            Some("agent_spawn")
        );
    }

    #[test]
    fn specialist_task_inherits_parent_continuation_as_resume_context() {
        let now = Utc::now();
        let parent_task = Task::new(AgentId("root".to_string()), "research startup.md");
        let parent_task_id = parent_task.id.clone();
        let parent_session_id = parent_task.session_id.clone();
        let continuation = sample_parent_continuation(&parent_task.description);
        let manifest = AgentManifest {
            agent_id: AgentId("specialist-research".to_string()),
            domain: "research".to_string(),
            status: AgentStatus::Idle,
            description: "research specialist".to_string(),
            role_prompt: None,
            initial_prompt: None,
            model_id: None,
            created_at: now,
            updated_at: now,
            parent_agent_id: Some(AgentId("root".to_string())),
            capabilities: vec!["research".to_string()],
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            required_mcp_servers: Vec::new(),
            authority: AgentAuthority::default(),
            lifecycle: AgentLifecycle::ready(),
            budget: AgentBudget::default(),
        };

        let task = build_specialist_task(
            &parent_task,
            Some(continuation.clone()),
            &manifest,
            "spawn_specialist",
        );

        let resume = task.resume_context.expect("specialist resume context");
        assert_eq!(resume.source_task_id, parent_task_id);
        assert_eq!(resume.source_session_id, parent_session_id);
        assert_eq!(resume.resume_reason, "delegated specialist sidechain");
        assert_eq!(resume.continuation_window.objective, continuation.objective);
        assert_eq!(resume.continuation_window.reannounced_artifacts.len(), 1);
    }

    #[test]
    fn missing_required_mcp_servers_reports_unavailable_servers() {
        let snapshot = McpRegistrySnapshot {
            servers: vec![
                McpServerSnapshot {
                    name: "docs".to_string(),
                    tools: Vec::new(),
                    resources: Vec::new(),
                    error: None,
                },
                McpServerSnapshot {
                    name: "broken".to_string(),
                    tools: Vec::new(),
                    resources: Vec::new(),
                    error: Some("connection failed".to_string()),
                },
            ],
        };

        let missing = missing_required_mcp_servers(
            &[
                "docs".to_string(),
                "github".to_string(),
                "broken".to_string(),
            ],
            &snapshot,
        );

        assert_eq!(missing, vec!["github".to_string(), "broken".to_string()]);
    }

    #[test]
    fn route_existing_manifest_refreshes_specialist_from_definition_file() {
        let (dir, service) = test_service();
        let now = Utc::now();
        let old_manifest = AgentManifest {
            agent_id: AgentId("specialist-research".to_string()),
            domain: "research".to_string(),
            status: AgentStatus::Idle,
            description: "Old description".to_string(),
            role_prompt: Some("Old role prompt".to_string()),
            initial_prompt: Some("Old initial prompt".to_string()),
            model_id: Some("old-model".to_string()),
            created_at: now,
            updated_at: now,
            parent_agent_id: Some(AgentId("root".to_string())),
            capabilities: vec!["research".to_string()],
            allowed_tools: vec!["read_file".to_string()],
            denied_tools: Vec::new(),
            required_mcp_servers: Vec::new(),
            authority: AgentAuthority::default(),
            lifecycle: AgentLifecycle::ready(),
            budget: AgentBudget::default(),
        };
        must(service.persist_manifest(&old_manifest));

        let definition_path = dir
            .path()
            .join("agents")
            .join("specialist-research")
            .join("definition.toml");
        std::fs::create_dir_all(
            definition_path
                .parent()
                .unwrap_or_else(|| panic!("definition path missing parent")),
        )
        .unwrap_or_else(|error| panic!("mkdir: {error}"));
        std::fs::write(
            &definition_path,
            r#"description = "Refreshed research specialist"
role_prompt = "You are the refreshed research specialist."
initial_prompt = "Use the refreshed first-turn directive."
model_id = "claude-opus-4-20250514"
capabilities = ["research", "refresh"]
allowed_tools = ["read_file", "search_text"]
denied_tools = ["run_command"]
required_mcp_servers = []
max_steps = 11
"#,
        )
        .unwrap_or_else(|error| panic!("write definition: {error}"));

        let refreshed = must(service.route_existing_manifest(
            &RouteAgentRequest {
                parent_task: Task::new(AgentId("root".to_string()), "research startup.md"),
                parent_continuation_window: None,
                decision: RoutingDecision::RouteToExisting(AgentId(
                    "specialist-research".to_string(),
                )),
            },
            &AgentId("specialist-research".to_string()),
        ));

        assert_eq!(refreshed.description, "Refreshed research specialist");
        assert_eq!(
            refreshed.role_prompt.as_deref(),
            Some("You are the refreshed research specialist.")
        );
        assert_eq!(
            refreshed.initial_prompt.as_deref(),
            Some("Use the refreshed first-turn directive.")
        );
        assert_eq!(
            refreshed.model_id.as_deref(),
            Some("claude-opus-4-20250514")
        );
        assert_eq!(refreshed.allowed_tools, vec!["read_file", "search_text"]);
        assert_eq!(refreshed.budget.max_steps_per_task, 11);
    }
}
