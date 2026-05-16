use retina_types::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

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
    pub(crate) state: &'a crate::TaskLoopState,
    pub(crate) control: Option<&'a ExecutionControlHandle>,
    pub(crate) current_step: usize,
    pub(crate) max_steps: usize,
}

pub(crate) struct ContextAssemblyInput<'a> {
    pub(crate) task: &'a Task,
    pub(crate) state: &'a crate::TaskLoopState,
    pub(crate) last_result: Option<String>,
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

pub struct ReflexEngine {
    rules: Mutex<Vec<ReflexiveRule>>,
}

impl ReflexEngine {
    pub fn new(rules: Vec<ReflexiveRule>) -> Self {
        Self {
            rules: Mutex::new(rules),
        }
    }

    pub fn check(&self, task: &Task, _intent: &Intent) -> Option<Action> {
        for rule in &*recover_mutex(&self.rules) {
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

    pub fn sync(&self, rules: Vec<ReflexiveRule>) {
        *recover_mutex(&self.rules) = rules;
    }
}

fn rule_action(rule: &ReflexiveRule) -> Option<Action> {
    match &rule.action {
        RuleAction::UseAction(action) => Some(action.clone()),
        _ => None,
    }
}

#[derive(Default)]
pub struct CircuitBreaker {
    failure_counts: Mutex<HashMap<String, usize>>,
}

impl CircuitBreaker {
    pub fn is_tripped(&self, intent: &Intent) -> bool {
        let key = intent.objective.clone();
        recover_mutex(&self.failure_counts)
            .get(&key)
            .copied()
            .unwrap_or_default()
            >= 3
    }

    pub fn record_failure(&self, intent: &Intent) {
        let key = intent.objective.clone();
        let mut counts = recover_mutex(&self.failure_counts);
        *counts.entry(key).or_insert(0) += 1;
    }
}

pub(crate) fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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
        ActionResult::TextSearch { matches, .. } => {
            if matches.is_empty() {
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
            {
                0.15
            } else {
                0.7
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

    let _ = framing;

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
