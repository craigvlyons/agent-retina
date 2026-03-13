use chrono::Utc;
use retina_traits::{Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Kernel {
    shell: Box<dyn Shell>,
    reasoner: Box<dyn Reasoner>,
    memory: Box<dyn Memory>,
    reflex_engine: ReflexEngine,
    circuit_breaker: CircuitBreaker,
    router: Router,
}

impl Kernel {
    pub fn new(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
    ) -> Result<Self> {
        let active_rules = memory.active_rules().unwrap_or_default();
        Ok(Self {
            shell,
            reasoner,
            memory,
            reflex_engine: ReflexEngine::new(active_rules),
            circuit_breaker: CircuitBreaker::default(),
            router: Router,
        })
    }

    pub fn route_task(&self, _task: &Task) -> RoutingDecision {
        self.router.route_task()
    }

    pub fn execute_task(&self, task: Task) -> Result<Outcome> {
        self.execute_task_with_config(task, ExecutionConfig::default())
    }

    pub fn execute_task_with_config(&self, task: Task, config: ExecutionConfig) -> Result<Outcome> {
        self.emit_event(EventSpec::new(
            &task,
            None,
            None,
            TimelineEventType::TaskReceived,
            json!({ "task": task.description }),
        ))?;

        match self.route_task(&task) {
            RoutingDecision::HandleDirectly => {}
            RoutingDecision::RouteToExisting(agent_id) => {
                return Ok(Outcome::Blocked(format!(
                    "routing to {} not available in v1",
                    agent_id
                )));
            }
            RoutingDecision::Reactivate(agent_id) => {
                return Ok(Outcome::Blocked(format!(
                    "reactivating {} not available in v1",
                    agent_id
                )));
            }
            RoutingDecision::SpawnSpecialist { domain, .. } => {
                return Ok(Outcome::Blocked(format!(
                    "spawning specialist for {domain} not available in v1"
                )));
            }
        }

        let mut intent = Intent::from_task(&task);
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::TaskContextAssembled,
            json!({ "route": "handle_directly" }),
        ))?;
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::IntentCreated,
            json!({ "objective": intent.objective }),
        ))?;

        let reflex_action = self.reflex_engine.check(&task, &intent);
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::ReflexChecked,
            json!({ "matched": reflex_action.is_some() }),
        ))?;

        let tripped = self.circuit_breaker.is_tripped(&intent);
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::CircuitBreakerChecked,
            json!({ "tripped": tripped }),
        ))?;
        if tripped {
            return Ok(Outcome::Blocked("circuit breaker is tripped".to_string()));
        }

        let mut state = TaskLoopState::new(config.max_steps);
        let mut next_reflex_action = reflex_action;

        loop {
            if state.step_index >= config.max_steps {
                let reason = format!("step budget exhausted after {} steps", config.max_steps);
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    None,
                    TimelineEventType::TaskFailed,
                    json!({ "reason": reason, "max_steps": config.max_steps }),
                ))?;
                return Ok(Outcome::Blocked(reason));
            }

            let step = self.select_action(
                &task,
                &intent,
                next_reflex_action.take(),
                state.last_result_json.clone(),
                state.step_index + 1,
                config.max_steps,
            )?;
            let outcome = self.execute_action(&task, &mut intent, &step, true)?;
            state.record_step(&step, &outcome)?;

            if step.task_complete || matches!(outcome, Outcome::Failure(_) | Outcome::Blocked(_)) {
                return Ok(outcome);
            }

            if config.pause_before_continuation && self.should_cancel_continuation()? {
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCancelled,
                    json!({ "reason": "cancelled by operator between steps" }),
                ))?;
                return Ok(Outcome::Blocked("task cancelled by operator".to_string()));
            }
        }
    }

    fn select_action(
        &self,
        task: &Task,
        intent: &Intent,
        reflex_action: Option<Action>,
        last_result: Option<String>,
        current_step: usize,
        max_steps: usize,
    ) -> Result<StepDecision> {
        if let Some(action) = reflex_action {
            self.emit_event(EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::ReflexSelected,
                json!({ "action": action_label(&action) }),
            ))?;
            return Ok(StepDecision {
                action,
                task_complete: true,
            });
        }

        let context = self.assemble_context(task, last_result, current_step, max_steps)?;
        let response = self.reasoner.reason(&ReasonRequest {
            tools: context.tools.clone(),
            context,
            constraints: self
                .shell
                .constraints()
                .iter()
                .map(|constraint| format!("{constraint:?}"))
                .collect(),
            max_tokens: Some(768),
        })?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&response.action),
            TimelineEventType::ReasonerCalled,
            json!({ "reasoning": response.reasoning, "tokens": response.tokens_used, "task_complete": response.task_complete }),
        ))?;
        Ok(StepDecision {
            action: response.action,
            task_complete: response.task_complete,
        })
    }

    fn execute_action(
        &self,
        task: &Task,
        intent: &mut Intent,
        step: &StepDecision,
        allow_retry: bool,
    ) -> Result<Outcome> {
        let action = step.action.clone();
        intent.action = Some(action.clone());
        intent.expects_change = action.expects_change();
        intent.hash_scope = action.hash_scope();

        let pre = self.shell.capture_state(&intent.hash_scope)?;
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::PreStateCaptured,
                json!({ "scope": intent.hash_scope }),
            )
            .with_pre_hash(pre.cwd_hash.clone()),
        )?;

        if action_requires_approval(&action) {
            let response = self.shell.request_approval(&ApprovalRequest {
                action: action_label(&action),
                reason: approval_reason(&action),
            })?;
            if matches!(response, ApprovalResponse::Denied) {
                return Err(KernelError::ApprovalDenied(
                    "command denied by operator".to_string(),
                ));
            }
        }

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ActionDispatched,
            json!({ "action": action_label(&action) }),
        ))?;

        let result = match self.shell.execute(&action) {
            Ok(result) => result,
            Err(error) => {
                self.circuit_breaker.record_failure(intent);
                return self.reflect_or_fail(task, intent, &action, error.to_string(), allow_retry);
            }
        };

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ActionResultReceived,
            json!({ "result": result }),
        ))?;

        let post = self.shell.capture_state(&intent.hash_scope)?;
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::PostStateCaptured,
                json!({ "scope": intent.hash_scope }),
            )
            .with_post_hash(post.cwd_hash.clone()),
        )?;

        let delta = self.shell.compare_state(&pre, &post, Some(&action))?;
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::StateDeltaComputed,
                json!({ "summary": delta.summary, "kind": delta.kind }),
            )
            .with_delta(delta.summary.clone()),
        )?;

        let experience_id = self.record_experience(task, intent, &action, &result, &delta)?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ExperiencePersisted,
            json!({ "experience_id": experience_id }),
        ))?;
        self.memory
            .update_utility(experience_id.clone(), delta.utility_score())?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::UtilityScored,
            json!({ "experience_id": experience_id, "utility": delta.utility_score() }),
        ))?;
        self.promote_reflex_if_ready(task, &action, &delta)?;

        if let Some(reason) = action_failure_reason(&result, &delta, &action) {
            self.circuit_breaker.record_failure(intent);
            return self.reflect_or_fail(task, intent, &action, reason, allow_retry);
        }

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::TaskStepCompleted,
            json!({ "result": "step_completed" }),
        ))?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::TaskCompleted,
            json!({ "outcome": "success" }),
        ))?;
        Ok(Outcome::Success(result))
    }

    fn reflect_or_fail(
        &self,
        task: &Task,
        intent: &mut Intent,
        action: &Action,
        reason: String,
        allow_retry: bool,
    ) -> Result<Outcome> {
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(action),
            TimelineEventType::ReflectionRequested,
            json!({ "reason": reason }),
        ))?;

        let reflection_context = self.assemble_context(task, Some(reason.clone()), 1, 1)?;
        let reflection = self.reasoner.reflect(&ReasonRequest {
            tools: reflection_context.tools.clone(),
            context: reflection_context,
            constraints: self
                .shell
                .constraints()
                .iter()
                .map(|constraint| format!("{constraint:?}"))
                .collect(),
            max_tokens: Some(384),
        })?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&reflection.action),
            TimelineEventType::ReflectionCompleted,
            json!({ "reasoning": reflection.reasoning, "retry": allow_retry, "task_complete": reflection.task_complete }),
        ))?;

        if allow_retry && should_retry(action, &reflection.action) {
            let retry_step = StepDecision {
                action: reflection.action,
                task_complete: reflection.task_complete,
            };
            return self.execute_action(task, intent, &retry_step, false);
        }

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(action),
            TimelineEventType::TaskFailed,
            json!({ "reason": reason }),
        ))?;
        Ok(Outcome::Failure(reason))
    }

    fn record_experience(
        &self,
        task: &Task,
        intent: &Intent,
        action: &Action,
        result: &ActionResult,
        delta: &StateDelta,
    ) -> Result<ExperienceId> {
        let experience = Experience {
            id: None,
            session_id: task.session_id.clone(),
            task_id: task.id.clone(),
            intent_id: intent.id.clone(),
            action_summary: action_label(action),
            outcome: format!("{:?}", delta.kind),
            utility: delta.utility_score(),
            created_at: Utc::now(),
            metadata: json!({
                "delta": delta.summary,
                "result": result,
            }),
        };
        self.memory.record_experience(&experience)
    }

    fn assemble_context(
        &self,
        task: &Task,
        last_result: Option<String>,
        current_step: usize,
        max_steps: usize,
    ) -> Result<AssembledContext> {
        let experiences = self.memory.recall_experiences(&task.description, 3)?;
        let knowledge = self.memory.recall_knowledge(&task.description, 3)?;
        let mut tools = default_tool_descriptors(self.shell.capabilities());
        tools.extend(
            self.memory
                .find_tools(&task.description)?
                .into_iter()
                .map(|tool| ToolDescriptor {
                    name: tool.name,
                    description: tool.description,
                }),
        );

        Ok(AssembledContext {
            identity: format!(
                "You are Retina/{}. Execute tasks through the CLI shell.",
                task.agent_id
            ),
            task: task.description.clone(),
            tools,
            memory_slice: experiences
                .into_iter()
                .map(|experience| {
                    format!(
                        "experience: {} ({})",
                        experience.action_summary, experience.outcome
                    )
                })
                .chain(
                    knowledge
                        .into_iter()
                        .map(|item| format!("knowledge: {}", item.content)),
                )
                .collect(),
            last_result,
            current_step,
            max_steps,
        })
    }

    fn should_cancel_continuation(&self) -> Result<bool> {
        let input = self
            .shell
            .request_input("Press Enter to continue to the next step or type /stop to cancel")?;
        let normalized = input.trim().to_lowercase();
        Ok(matches!(normalized.as_str(), "/stop" | "stop" | "/cancel" | "cancel"))
    }

    fn promote_reflex_if_ready(
        &self,
        task: &Task,
        action: &Action,
        delta: &StateDelta,
    ) -> Result<()> {
        if !matches!(delta.kind, StateDeltaKind::ChangedAsExpected) {
            return Ok(());
        }
        if matches!(
            action,
            Action::Respond { .. }
                | Action::RecordNote { .. }
                | Action::InspectWorkingDirectory { .. }
        ) {
            return Ok(());
        }

        let action_summary = action_label(action);
        let successful_repeats = self
            .memory
            .recall_experiences(&task.description, 10)?
            .into_iter()
            .filter(|experience| {
                experience.action_summary == action_summary && experience.utility > 0.0
            })
            .count();
        if successful_repeats < 2 {
            return Ok(());
        }

        let rule = ReflexiveRule {
            id: None,
            name: format!("promoted:{}", task.description),
            condition: RuleCondition::TaskContains(task.description.clone()),
            action: RuleAction::UseAction(action.clone()),
            confidence: 0.6,
            active: true,
            last_fired: None,
        };
        let _ = self.memory.store_rule(&rule)?;
        self.reflex_engine.promote(rule);
        Ok(())
    }

    fn emit_event(&self, spec: EventSpec<'_>) -> Result<()> {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: spec.task.session_id.clone(),
            task_id: spec.task.id.clone(),
            agent_id: spec.task.agent_id.clone(),
            timestamp: Utc::now(),
            event_type: spec.event_type,
            intent_id: spec.intent.map(|intent| intent.id.clone()),
            action_id: spec.action.map(Action::id),
            pre_state_hash: spec.pre_state_hash,
            post_state_hash: spec.post_state_hash,
            delta_summary: spec.delta_summary,
            duration_ms: spec.duration_ms,
            payload_json: spec.payload_json,
        };
        self.memory.append_timeline_event(&event)
    }
}

#[derive(Clone)]
struct StepDecision {
    action: Action,
    task_complete: bool,
}

struct TaskLoopState {
    step_index: usize,
    last_result_json: Option<String>,
}

impl TaskLoopState {
    fn new(_max_steps: usize) -> Self {
        Self {
            step_index: 0,
            last_result_json: None,
        }
    }

    fn record_step(&mut self, step: &StepDecision, outcome: &Outcome) -> Result<()> {
        self.step_index += 1;
        self.last_result_json = match outcome {
            Outcome::Success(result) if !matches!(step.action, Action::Respond { .. }) => {
                Some(
                    serde_json::to_string(result)
                        .map_err(|error| KernelError::Reasoning(error.to_string()))?,
                )
            }
            _ => None,
        };
        Ok(())
    }
}

struct EventSpec<'a> {
    task: &'a Task,
    intent: Option<&'a Intent>,
    action: Option<&'a Action>,
    event_type: TimelineEventType,
    payload_json: serde_json::Value,
    pre_state_hash: Option<String>,
    post_state_hash: Option<String>,
    delta_summary: Option<String>,
    duration_ms: Option<u64>,
}

impl<'a> EventSpec<'a> {
    fn new(
        task: &'a Task,
        intent: Option<&'a Intent>,
        action: Option<&'a Action>,
        event_type: TimelineEventType,
        payload_json: serde_json::Value,
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

    fn with_pre_hash(mut self, hash: String) -> Self {
        if !hash.is_empty() {
            self.pre_state_hash = Some(hash);
        }
        self
    }

    fn with_post_hash(mut self, hash: String) -> Self {
        if !hash.is_empty() {
            self.post_state_hash = Some(hash);
        }
        self
    }

    fn with_delta(mut self, summary: String) -> Self {
        self.delta_summary = Some(summary);
        self
    }
}

pub struct Router;

impl Router {
    fn route_task(&self) -> RoutingDecision {
        RoutingDecision::HandleDirectly
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
        for rule in &*self.rules.lock().expect("reflex engine mutex poisoned") {
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

    pub fn promote(&self, rule: ReflexiveRule) {
        let mut rules = self.rules.lock().expect("reflex engine mutex poisoned");
        let already_present = rules.iter().any(|existing| existing.name == rule.name);
        if !already_present {
            rules.push(rule);
        }
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
        self.failure_counts
            .lock()
            .expect("circuit breaker mutex poisoned")
            .get(&key)
            .copied()
            .unwrap_or_default()
            >= 3
    }

    pub fn record_failure(&self, intent: &Intent) {
        let key = intent.objective.clone();
        let mut counts = self
            .failure_counts
            .lock()
            .expect("circuit breaker mutex poisoned");
        *counts.entry(key).or_insert(0) += 1;
    }
}

fn action_requires_approval(action: &Action) -> bool {
    match action {
        Action::RunCommand {
            require_approval: true,
            ..
        }
        | Action::WriteFile {
            require_approval: true,
            ..
        }
        | Action::AppendFile {
            require_approval: true,
            ..
        } => true,
        _ => false,
    }
}

fn approval_reason(action: &Action) -> String {
    match action {
        Action::RunCommand { .. } => "command requires explicit approval".to_string(),
        Action::WriteFile { path, .. } => {
            format!("writing {} modifies the filesystem", path.display())
        }
        Action::AppendFile { path, .. } => {
            format!("appending to {} modifies the filesystem", path.display())
        }
        _ => "operator approval required".to_string(),
    }
}

fn action_failure_reason(
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

fn should_retry(previous: &Action, next: &Action) -> bool {
    action_label(previous) != action_label(next) && !matches!(next, Action::Respond { .. })
}

fn default_tool_descriptors(capabilities: ShellCapabilities) -> Vec<ToolDescriptor> {
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
            description: "Read file contents with truncation protection.".to_string(),
        });
    }
    if capabilities.can_write_files {
        tools.push(ToolDescriptor {
            name: "write_file".to_string(),
            description: "Write or append files with approval and state verification.".to_string(),
        });
    }
    if capabilities.can_execute_commands {
        tools.push(ToolDescriptor {
            name: "run_command".to_string(),
            description: "Run a controlled shell command when structured actions are not enough."
                .to_string(),
        });
    }

    tools
}

fn action_label(action: &Action) -> String {
    match action {
        Action::RunCommand { command, .. } => format!("run_command:{command}"),
        Action::InspectPath { path, .. } => format!("inspect_path:{}", path.display()),
        Action::InspectWorkingDirectory { .. } => "inspect_working_directory".to_string(),
        Action::ListDirectory {
            path, recursive, ..
        } => {
            format!("list_directory:{}:recursive={recursive}", path.display())
        }
        Action::FindFiles { root, pattern, .. } => {
            format!("find_files:{}:{pattern}", root.display())
        }
        Action::SearchText { root, query, .. } => format!("search_text:{}:{query}", root.display()),
        Action::ReadFile { path, .. } => format!("read_file:{}", path.display()),
        Action::WriteFile { path, .. } => format!("write_file:{}", path.display()),
        Action::AppendFile { path, .. } => format!("append_file:{}", path.display()),
        Action::RecordNote { note, .. } => format!("record_note:{note}"),
        Action::Respond { message, .. } => format!("respond:{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_test_utils::{MockMemory, MockReasoner, MockShell};

    #[test]
    fn router_defaults_to_handle_directly() {
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::Respond {
                id: ActionId::new(),
                message: "hello".to_string(),
            })),
            Box::new(MockMemory::default()),
        )
        .unwrap();
        let task = Task::new(AgentId::new(), "inspect");
        assert!(matches!(
            kernel.route_task(&task),
            RoutingDecision::HandleDirectly
        ));
    }

    #[test]
    fn execute_loop_records_timeline() {
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::Respond {
                id: ActionId::new(),
                message: "hello".to_string(),
            })),
            Box::new(MockMemory::default()),
        )
        .unwrap();
        let task = Task::new(AgentId::new(), "hello");
        let outcome = kernel.execute_task(task).unwrap();
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));
    }

    #[test]
    fn unchanged_mutating_action_triggers_reflection() {
        let shell = MockShell::default().with_force_unchanged(true);
        let action = Action::RunCommand {
            id: ActionId::new(),
            command: "echo hi > note.txt".to_string(),
            cwd: None,
            require_approval: false,
            expect_change: true,
            state_scope: HashScope::default(),
        };
        let kernel = Kernel::new(
            Box::new(shell),
            Box::new(MockReasoner::for_action(action.clone())),
            Box::new(MockMemory::default()),
        )
        .unwrap();
        let task = Task::new(AgentId::new(), "run echo hi > note.txt");
        let outcome = kernel.execute_task(task).unwrap();
        assert!(matches!(outcome, Outcome::Failure(_)));
    }

    #[test]
    fn repeated_successful_pattern_promotes_rule() {
        let memory = MockMemory::default();
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::ReadFile {
                id: ActionId::new(),
                path: "startup.md".into(),
                max_bytes: None,
            })),
            Box::new(memory.clone()),
        )
        .unwrap();

        let task = "read startup.md";
        let _ = kernel
            .execute_task(Task::new(AgentId::new(), task))
            .unwrap();
        let _ = kernel
            .execute_task(Task::new(AgentId::new(), task))
            .unwrap();

        assert!(memory.rule_count() >= 1);
    }
}
