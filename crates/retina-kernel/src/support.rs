use retina_types::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

#[derive(Clone)]
pub(crate) struct StepDecision {
    pub(crate) action: Action,
    pub(crate) task_complete: bool,
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
    pub(crate) last_result_summary: Option<String>,
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
        ActionResult::DirectoryListing { entries, .. } => {
            if entries.is_empty() {
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
        ActionResult::TextSearch { matches, .. } => {
            if matches.is_empty() {
                0.1
            } else {
                0.65
            }
        }
        ActionResult::FileWrite { .. } => 1.0,
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

pub(crate) fn default_tool_descriptors(capabilities: ShellCapabilities) -> Vec<ToolDescriptor> {
    let mut tools = vec![
        ToolDescriptor {
            name: "respond".to_string(),
            description: "Answer operator questions directly when no shell action is needed."
                .to_string(),
        },
        ToolDescriptor {
            name: "inspect_path".to_string(),
            description: "Inspect one path for existence, metadata, and optional content hash."
                .to_string(),
        },
        ToolDescriptor {
            name: "list_directory".to_string(),
            description: "List files and directories in a target directory.".to_string(),
        },
        ToolDescriptor {
            name: "find_files".to_string(),
            description: "Find files by filename or path fragment.".to_string(),
        },
        ToolDescriptor {
            name: "search_text".to_string(),
            description: "Search text content across files in the current workspace.".to_string(),
        },
    ];

    if capabilities.can_read_files {
        tools.push(ToolDescriptor {
            name: "read_file".to_string(),
            description: "Read text-like files such as markdown, code, config, and plaintext with truncation protection.".to_string(),
        });
    }
    if capabilities.can_extract_documents {
        tools.push(ToolDescriptor {
            name: "extract_document_text".to_string(),
            description: "Extract readable text from documents such as PDFs when raw file reads would be binary or unhelpful.".to_string(),
        });
    }
    if capabilities.can_write_files {
        tools.push(ToolDescriptor {
            name: "write_file".to_string(),
            description: "Create, overwrite, or append local files and verify the result."
                .to_string(),
        });
    }
    if capabilities.can_execute_commands {
        tools.push(ToolDescriptor {
            name: "run_command".to_string(),
            description:
                "Run shell commands, pipelines, or local scripts when they best advance the task."
                    .to_string(),
        });
    }

    tools
}
