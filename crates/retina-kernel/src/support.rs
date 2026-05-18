use retina_types::*;
use serde_json::Value;
use std::path::Path;

#[derive(Clone)]
pub(crate) struct StepDecision {
    pub(crate) action: Action,
    pub(crate) task_complete: bool,
    pub(crate) framing: Option<ReasonerTaskFraming>,
}

pub(crate) enum ActionExecution {
    Outcome(Outcome),
}

pub(crate) struct StepSelectionContext<'a> {
    pub(crate) task: &'a Task,
    pub(crate) intent: &'a Intent,
    pub(crate) state: &'a mut crate::TaskLoopState,
    pub(crate) control: Option<&'a ExecutionControlHandle>,
    pub(crate) current_step: usize,
    pub(crate) max_steps: usize,
}

pub(crate) struct ContextAssemblyInput<'a> {
    pub(crate) task: &'a Task,
    pub(crate) state: &'a crate::TaskLoopState,
    pub(crate) operator_guidance: Option<String>,
    pub(crate) current_step: usize,
    pub(crate) max_steps: usize,
}

pub(crate) struct EventSpec<'a> {
    pub(crate) task: &'a Task,
    pub(crate) intent: Option<&'a Intent>,
    pub(crate) action: Option<&'a Action>,
    pub(crate) event_type: TimelineEventType,
    pub(crate) payload_json: Value,
    pub(crate) pre_state_hash: Option<String>,
    pub(crate) post_state_hash: Option<String>,
    pub(crate) delta_summary: Option<String>,
    pub(crate) duration_ms: Option<u64>,
}

impl<'a> EventSpec<'a> {
    pub(crate) fn new(
        task: &'a Task,
        intent: Option<&'a Intent>,
        action: Option<&'a Action>,
        event_type: TimelineEventType,
        payload_json: Value,
    ) -> Self {
        Self {
            task,
            intent,
            action,
            event_type,
            payload_json,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
        }
    }

    pub(crate) fn with_pre_hash(mut self, hash: String) -> Self {
        if !hash.is_empty() {
            self.pre_state_hash = Some(hash);
        }
        self
    }

    pub(crate) fn with_post_hash(mut self, hash: String) -> Self {
        if !hash.is_empty() {
            self.post_state_hash = Some(hash);
        }
        self
    }

    pub(crate) fn with_delta(mut self, summary: String) -> Self {
        self.delta_summary = Some(summary);
        self
    }
}

pub(crate) fn select_reflex_action(task: &Task, rules: &[ReflexiveRule]) -> Option<Action> {
    for rule in rules {
        if !rule.active {
            continue;
        }
        match &rule.condition {
            RuleCondition::Always => return rule_action(rule),
            RuleCondition::TaskContains(text) if task.description.contains(text) => {
                return rule_action(rule);
            }
            _ => {}
        }
    }
    None
}

fn rule_action(rule: &ReflexiveRule) -> Option<Action> {
    match &rule.action {
        RuleAction::UseAction(action) => Some(action.clone()),
        _ => None,
    }
}

pub(crate) fn action_requires_approval(action: &Action) -> bool {
    action.approval_required_by_policy()
}

pub(crate) fn approval_reason(action: &Action) -> String {
    match action {
        Action::RunCommand { command, .. } => match classify_privileged_command(command) {
            Some(PrivilegedCommandKind::Delete) => {
                "delete-like command requires explicit approval".to_string()
            }
            Some(PrivilegedCommandKind::Kill) => {
                "kill-like command requires explicit approval".to_string()
            }
            None => "operator approval required".to_string(),
        },
        _ => "operator approval required".to_string(),
    }
}

pub(crate) fn action_failure_reason(
    result: &ActionResult,
    delta: &StateDelta,
    action: &Action,
) -> Option<String> {
    if let ActionResult::Command(command) = result {
        if !command.success {
            return Some(format!(
                "command failed with exit {:?}: {}",
                command.exit_code,
                command.stderr.trim()
            ));
        }
    }

    if let ActionResult::DelegatedTask(result) = result {
        if result.status != DelegatedTaskStatus::Completed {
            return Some(format!(
                "delegated child agent {} ended with {:?}: {}",
                result.agent_id, result.status, result.summary
            ));
        }
    }

    if let ActionResult::McpToolCall(result) = result {
        if result.is_error {
            return Some(format!(
                "MCP tool {} on {} returned an error: {}",
                result.tool, result.server, result.content_preview
            ));
        }
    }

    if action.expects_change()
        && matches!(
            delta.kind,
            StateDeltaKind::Unchanged | StateDeltaKind::ChangedUnexpectedly
        )
    {
        return Some(delta.summary.clone());
    }

    None
}

pub(crate) fn action_utility(action: &Action, result: &ActionResult, delta: &StateDelta) -> f64 {
    if action.expects_change() {
        return delta.utility_score();
    }

    match result {
        ActionResult::Command(command) => {
            if command.success {
                0.6
            } else {
                -1.0
            }
        }
        ActionResult::Inspection(state) => {
            if state.files.is_empty() {
                0.25
            } else {
                0.45
            }
        }
        ActionResult::DirectoryListing { summary, .. } => {
            if summary.total_entries == 0 {
                0.15
            } else {
                0.55
            }
        }
        ActionResult::FileMatches { matches, .. } => {
            if matches.is_empty() {
                0.1
            } else {
                0.6
            }
        }
        ActionResult::FileRead {
            content, truncated, ..
        }
        | ActionResult::DocumentText {
            content, truncated, ..
        } => {
            if content.trim().is_empty() {
                0.05
            } else if *truncated {
                0.65
            } else {
                0.85
            }
        }
        ActionResult::StructuredData {
            rows, truncated, ..
        } => {
            if rows.is_empty() {
                0.05
            } else if *truncated {
                0.65
            } else {
                0.85
            }
        }
        ActionResult::TextSearch {
            matches,
            filenames,
            content,
            num_matches,
            ..
        } => {
            if matches.is_empty()
                && filenames.is_empty()
                && content.as_deref().unwrap_or_default().is_empty()
                && *num_matches == 0
            {
                0.1
            } else {
                0.65
            }
        }
        ActionResult::McpResources { resources, .. } => {
            if resources.is_empty() {
                0.2
            } else {
                0.55
            }
        }
        ActionResult::McpResourceRead(result) => {
            if result.contents.is_empty() {
                0.1
            } else {
                0.75
            }
        }
        ActionResult::McpToolCall(result) => {
            if result.is_error {
                -0.75
            } else if result.content_preview.trim().is_empty()
                && result.structured_content.is_none()
                && result.search_hits.is_empty()
            {
                0.15
            } else {
                match result.search_outcome_kind.as_ref() {
                    Some(McpSearchOutcomeKind::SingleEvent) => 0.9,
                    Some(McpSearchOutcomeKind::SpecificListing) => 0.8,
                    Some(McpSearchOutcomeKind::NewsRoundup) => 0.55,
                    Some(McpSearchOutcomeKind::GenericPortal) => 0.45,
                    Some(McpSearchOutcomeKind::NoLocalSignal) => 0.25,
                    Some(McpSearchOutcomeKind::ValidationError) => -0.4,
                    Some(McpSearchOutcomeKind::ToolError) => -0.75,
                    Some(McpSearchOutcomeKind::NonSearchResult) | None => 0.7,
                }
            }
        }
        ActionResult::FileWrite { .. } => 1.0,
        ActionResult::DelegatedTask(result) => match result.status {
            DelegatedTaskStatus::Completed => 0.85,
            DelegatedTaskStatus::Failed => -1.0,
            DelegatedTaskStatus::Blocked => -0.4,
            DelegatedTaskStatus::Killed => -0.7,
        },
        ActionResult::NoteRecorded { .. } => 0.3,
        ActionResult::Response { message } => {
            if message.trim().is_empty() {
                0.0
            } else {
                0.25
            }
        }
    }
}

pub(crate) fn tool_authored_completion_message(
    result: &ActionResult,
    framing: Option<&ReasonerTaskFraming>,
) -> Option<String> {
    let ActionResult::FileWrite {
        path,
        mutation_kind,
        preview_excerpt,
        ..
    } = result
    else {
        return None;
    };

    if !framing_matches_written_artifact(path, framing) {
        return None;
    }

    let base = match mutation_kind {
        FileMutationKind::Create => {
            format!("File created successfully at: {}", path.display())
        }
        FileMutationKind::Overwrite | FileMutationKind::Append | FileMutationKind::ExactEdit => {
            format!("The file {} has been updated successfully.", path.display())
        }
        FileMutationKind::NotebookReplace
        | FileMutationKind::NotebookInsert
        | FileMutationKind::NotebookDelete => {
            format!(
                "The notebook {} has been updated successfully.",
                path.display()
            )
        }
    };

    let preview = preview_excerpt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("\nPreview: {value}"))
        .unwrap_or_default();

    Some(format!("{base}{preview}"))
}

pub(crate) fn completion_blocker_reason(
    task: &Task,
    working_sources: &[WorkingSource],
) -> Option<String> {
    if !task_requests_full_batch_coverage(&task.description) {
        return None;
    }

    let discovered_inputs = discovered_batch_input_paths(working_sources);
    if discovered_inputs.is_empty() {
        return None;
    }
    let covered_inputs = covered_batch_input_paths(working_sources);
    let missing = discovered_inputs
        .into_iter()
        .filter(|path| !covered_inputs.contains(path))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return None;
    }

    let preview = missing
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "the task asked for complete batch coverage, but {} discovered input(s) are still not covered: {}",
        missing.len(),
        preview
    ))
}

fn framing_matches_written_artifact(path: &Path, framing: Option<&ReasonerTaskFraming>) -> bool {
    let Some(framing) = framing else {
        return false;
    };
    if !matches!(framing.intent_kind, Some(TaskKind::Output)) {
        return false;
    }
    let Some(deliverable) = framing
        .deliverable
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    let deliverable_lower = deliverable.to_ascii_lowercase();
    let path_display = path.display().to_string().to_ascii_lowercase();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    let deliverable_file_name = Path::new(deliverable)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    deliverable_lower == path_display
        || path_display.ends_with(&format!("/{}", deliverable_lower))
        || matches!(
            (file_name.as_deref(), deliverable_file_name.as_deref()),
            (Some(path_name), Some(deliverable_name)) if path_name == deliverable_name
        )
        || matches!(
            file_name.as_deref(),
            Some(path_name) if path_name == deliverable_lower
        )
}

fn task_requests_full_batch_coverage(description: &str) -> bool {
    let normalized = description.to_ascii_lowercase();
    let mentions_all = normalized.contains("all pdf")
        || normalized.contains("all the pdf")
        || normalized.contains("all files")
        || normalized.contains("all the files")
        || normalized.contains("review all")
        || normalized.contains("read all")
        || normalized.contains("go through all");
    let mentions_container = normalized.contains("folder")
        || normalized.contains("directory")
        || normalized.contains("bulk-pdf");
    mentions_all && mentions_container
}

fn discovered_batch_input_paths(
    working_sources: &[WorkingSource],
) -> std::collections::BTreeSet<String> {
    let mut paths = std::collections::BTreeSet::new();
    for source in working_sources {
        if source.kind == "file"
            && source.role == "candidate"
            && looks_like_batch_input(&source.locator)
        {
            paths.insert(source.locator.clone());
        }
        if source.kind == "directory" && source.status == "listed" {
            for locator in &source.evidence_refs {
                if looks_like_batch_input(locator) {
                    paths.insert(locator.clone());
                }
            }
        }
    }
    paths
}

fn covered_batch_input_paths(
    working_sources: &[WorkingSource],
) -> std::collections::BTreeSet<String> {
    working_sources
        .iter()
        .filter(|source| source.role == "authoritative" && looks_like_batch_input(&source.locator))
        .map(|source| source.locator.clone())
        .collect()
}

fn looks_like_batch_input(locator: &str) -> bool {
    let lower = locator.to_ascii_lowercase();
    lower.ends_with(".pdf")
        || lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower.ends_with(".csv")
}
