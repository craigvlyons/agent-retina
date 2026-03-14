mod router;

use crate::router::Router;
use chrono::Utc;
use retina_traits::{Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

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
        Self::new_with_registry(shell, reasoner, memory, AgentRegistrySnapshot::default())
    }

    pub fn new_with_registry(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
        registry: AgentRegistrySnapshot,
    ) -> Result<Self> {
        let active_rules = memory.active_rules().unwrap_or_default();
        Ok(Self {
            shell,
            reasoner,
            memory,
            reflex_engine: ReflexEngine::new(active_rules),
            circuit_breaker: CircuitBreaker::default(),
            router: Router::v1(registry),
        })
    }

    pub fn route_task(&self, _task: &Task) -> RoutingDecision {
        self.router.route_task(_task).effective_decision
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

        let routing = self.router.route_task(&task);

        match routing.effective_decision.clone() {
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
            json!({
                "route": format!("{:?}", routing.effective_decision),
                "recommended_route": format!("{:?}", routing.recommended_decision),
                "routing_rationale": routing.rationale,
                "routing_candidates": routing.candidates,
                "network_enabled": routing.network_enabled
            }),
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

            if let Some(outcome) = self.check_cancellation(
                &task,
                Some(&intent),
                None,
                config.control.as_ref(),
                "before planning",
            )? {
                return Ok(outcome);
            }

            let step = self.select_action(
                &task,
                &intent,
                next_reflex_action.take(),
                &state,
                config.control.as_ref(),
                state.step_index + 1,
                config.max_steps,
            )?;
            if let Some(outcome) = self.check_cancellation(
                &task,
                Some(&intent),
                Some(&step.action),
                config.control.as_ref(),
                "before action dispatch",
            )? {
                return Ok(outcome);
            }
            let outcome =
                self.execute_action(&task, &mut intent, &step, config.control.as_ref(), true)?;
            let progress = state.record_step(&step, &outcome)?;

            if progress.repeated_without_progress {
                return self.reflect_or_fail(
                    &task,
                    &mut intent,
                    &step.action,
                    config.control.as_ref(),
                    "repeated the same step without discovering new information".to_string(),
                    true,
                );
            }

            if step.task_complete || matches!(outcome, Outcome::Failure(_) | Outcome::Blocked(_)) {
                return Ok(outcome);
            }
        }
    }

    fn select_action(
        &self,
        task: &Task,
        intent: &Intent,
        reflex_action: Option<Action>,
        state: &TaskLoopState,
        control: Option<&ExecutionControlHandle>,
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

        let context = self.assemble_context(
            task,
            state.last_result_json.clone(),
            state.last_result_summary.clone(),
            state.recent_steps.clone(),
            control.and_then(ExecutionControlHandle::take_guidance),
            current_step,
            max_steps,
        )?;
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
            json!({
                "action": action_label(&response.action),
                "reasoning": response.reasoning,
                "tokens": response.tokens_used,
                "task_complete": response.task_complete
            }),
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
        control: Option<&ExecutionControlHandle>,
        allow_retry: bool,
    ) -> Result<Outcome> {
        let action = step.action.clone();
        intent.action = Some(action.clone());
        intent.expects_change = action.expects_change();
        intent.hash_scope = action.hash_scope();

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "before pre-state capture",
        )? {
            return Ok(outcome);
        }

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
            if let Some(outcome) = self.check_cancellation(
                task,
                Some(intent),
                Some(&action),
                control,
                "before approval prompt",
            )? {
                return Ok(outcome);
            }
            let response = self.shell.request_approval(&ApprovalRequest {
                action: action_label(&action),
                reason: approval_reason(&action),
            })?;
            if matches!(response, ApprovalResponse::Cancelled) {
                return Ok(self.cancel_outcome(
                    task,
                    Some(intent),
                    Some(&action),
                    "task cancelled by operator during approval",
                )?);
            }
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

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "before action execution",
        )? {
            return Ok(outcome);
        }

        let result = match self.shell.execute_controlled(&action, control) {
            Ok(result) => result,
            Err(error) => {
                self.circuit_breaker.record_failure(intent);
                return self.reflect_or_fail(
                    task,
                    intent,
                    &action,
                    control,
                    error.to_string(),
                    allow_retry,
                );
            }
        };

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ActionResultReceived,
            json!({ "result": result }),
        ))?;

        if let ActionResult::Command(command) = &result {
            if command.cancelled {
                return Ok(self.cancel_outcome(
                    task,
                    Some(intent),
                    Some(&action),
                    command
                        .termination
                        .clone()
                        .unwrap_or_else(|| "task cancelled by operator".to_string()),
                )?);
            }
        }

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "after action result",
        )? {
            return Ok(outcome);
        }

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

        let utility = action_utility(&action, &result, &delta);
        let experience_id =
            self.record_experience(task, intent, &action, &result, &delta, utility)?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ExperiencePersisted,
            json!({ "experience_id": experience_id }),
        ))?;
        self.memory.update_utility(experience_id.clone(), utility)?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::UtilityScored,
            json!({ "experience_id": experience_id, "utility": utility }),
        ))?;
        let consolidation = self.memory.consolidate(&ConsolidationConfig::default())?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ConsolidationCompleted,
            json!({
                "merged_knowledge": consolidation.merged_knowledge,
                "promoted_rules": consolidation.promoted_rules,
                "compacted_events": consolidation.compacted_events
            }),
        ))?;
        if consolidation.promoted_rules > 0 {
            self.reflex_engine.sync(self.memory.active_rules()?);
        }

        if let Some(reason) = action_failure_reason(&result, &delta, &action) {
            self.circuit_breaker.record_failure(intent);
            return self.reflect_or_fail(task, intent, &action, control, reason, allow_retry);
        }

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::TaskStepCompleted,
            json!({ "result": "step_completed" }),
        ))?;
        if step.task_complete {
            self.emit_event(EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::TaskCompleted,
                json!({ "outcome": "success" }),
            ))?;
        }
        Ok(Outcome::Success(result))
    }

    fn reflect_or_fail(
        &self,
        task: &Task,
        intent: &mut Intent,
        action: &Action,
        control: Option<&ExecutionControlHandle>,
        reason: String,
        allow_retry: bool,
    ) -> Result<Outcome> {
        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(action),
            control,
            "before reflection",
        )? {
            return Ok(outcome);
        }
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(action),
            TimelineEventType::ReflectionRequested,
            json!({ "reason": reason }),
        ))?;

        let reflection_context = self.assemble_context(
            task,
            Some(reason.clone()),
            Some(reason.clone()),
            vec![format!("failed action: {}", action_label(action))],
            control.and_then(ExecutionControlHandle::take_guidance),
            1,
            1,
        )?;
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
            json!({
                "action": action_label(&reflection.action),
                "reasoning": reflection.reasoning,
                "retry": allow_retry,
                "task_complete": reflection.task_complete
            }),
        ))?;

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(action),
            control,
            "after reflection",
        )? {
            return Ok(outcome);
        }

        if allow_retry && should_retry(action, &reflection.action) {
            let retry_step = StepDecision {
                action: reflection.action,
                task_complete: reflection.task_complete,
            };
            return self.execute_action(task, intent, &retry_step, control, false);
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
        utility: f64,
    ) -> Result<ExperienceId> {
        let experience = Experience {
            id: None,
            session_id: task.session_id.clone(),
            task_id: task.id.clone(),
            intent_id: intent.id.clone(),
            action_summary: action_label(action),
            outcome: format!("{:?}", delta.kind),
            utility,
            created_at: Utc::now(),
            metadata: json!({
                "task": task.description,
                "action": action,
                "delta_kind": delta.kind,
                "delta": delta.summary,
                "result": result,
                "utility": utility,
            }),
        };
        self.memory.record_experience(&experience)
    }

    fn assemble_context(
        &self,
        task: &Task,
        last_result: Option<String>,
        last_result_summary: Option<String>,
        recent_steps: Vec<String>,
        operator_guidance: Option<String>,
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
                    let prior_task = experience
                        .metadata
                        .get("task")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    format!(
                        "experience: task={} action={} outcome={} utility={:.2}",
                        prior_task,
                        experience.action_summary,
                        experience.outcome,
                        experience.utility
                    )
                })
                .chain(knowledge.into_iter().map(|item| {
                    format!(
                        "knowledge: {} (confidence {:.2})",
                        item.content, item.confidence
                    )
                }))
                .collect(),
            last_result,
            last_result_summary,
            recent_steps,
            operator_guidance,
            current_step,
            max_steps,
        })
    }

    fn check_cancellation(
        &self,
        task: &Task,
        intent: Option<&Intent>,
        action: Option<&Action>,
        control: Option<&ExecutionControlHandle>,
        checkpoint: &str,
    ) -> Result<Option<Outcome>> {
        if control
            .map(ExecutionControlHandle::is_cancel_requested)
            .unwrap_or(false)
        {
            return Ok(Some(self.cancel_outcome(task, intent, action, checkpoint)?));
        }
        Ok(None)
    }

    fn cancel_outcome(
        &self,
        task: &Task,
        intent: Option<&Intent>,
        action: Option<&Action>,
        reason: impl Into<String>,
    ) -> Result<Outcome> {
        let reason = reason.into();
        self.emit_event(EventSpec::new(
            task,
            intent,
            action,
            TimelineEventType::TaskCancelRequested,
            json!({ "reason": reason }),
        ))?;
        self.emit_event(EventSpec::new(
            task,
            intent,
            action,
            TimelineEventType::TaskCancelled,
            json!({ "reason": reason }),
        ))?;
        Ok(Outcome::Blocked("task cancelled by operator".to_string()))
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
    last_result_summary: Option<String>,
    recent_steps: Vec<String>,
    seen_signatures: HashMap<String, usize>,
}

impl TaskLoopState {
    fn new(_max_steps: usize) -> Self {
        Self {
            step_index: 0,
            last_result_json: None,
            last_result_summary: None,
            recent_steps: Vec::new(),
            seen_signatures: HashMap::new(),
        }
    }

    fn record_step(&mut self, step: &StepDecision, outcome: &Outcome) -> Result<StepProgress> {
        self.step_index += 1;
        let mut repeated_without_progress = false;
        self.last_result_json = match outcome {
            Outcome::Success(result) if !matches!(step.action, Action::Respond { .. }) => {
                let summary = summarize_action_result(result);
                self.last_result_summary = Some(summary.clone());
                self.recent_steps.push(format!(
                    "step {}: {} -> {}",
                    self.step_index,
                    action_label(&step.action),
                    summary
                ));
                trim_recent_steps(&mut self.recent_steps);
                if let Some(signature) = repeated_step_signature(&step.action, result) {
                    let count = self.seen_signatures.entry(signature).or_insert(0);
                    *count += 1;
                    repeated_without_progress = *count > 1;
                }
                Some(
                    compact_action_result_for_context(result)
                        .map_err(|error| KernelError::Reasoning(error.to_string()))?,
                )
            }
            Outcome::Success(_) => {
                self.last_result_summary = Some("responded to operator".to_string());
                self.recent_steps.push(format!(
                    "step {}: {} -> responded to operator",
                    self.step_index,
                    action_label(&step.action)
                ));
                trim_recent_steps(&mut self.recent_steps);
                None
            }
            Outcome::Failure(reason) | Outcome::Blocked(reason) => {
                self.last_result_summary = Some(reason.clone());
                self.recent_steps.push(format!(
                    "step {}: {} -> {}",
                    self.step_index,
                    action_label(&step.action),
                    reason
                ));
                trim_recent_steps(&mut self.recent_steps);
                None
            }
        };
        Ok(StepProgress {
            repeated_without_progress,
        })
    }
}

#[derive(Default)]
struct StepProgress {
    repeated_without_progress: bool,
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

    pub fn promote(&self, rule: ReflexiveRule) {
        let mut rules = recover_mutex(&self.rules);
        let already_present = rules.iter().any(|existing| existing.name == rule.name);
        if !already_present {
            rules.push(rule);
        }
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

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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

fn action_utility(action: &Action, result: &ActionResult, delta: &StateDelta) -> f64 {
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

fn summarize_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Command(command) => format!(
            "command {} with exit {:?}",
            if command.success {
                "succeeded"
            } else {
                "failed"
            },
            command.exit_code
        ),
        ActionResult::Inspection(world) => format!("inspected {} path(s)", world.files.len()),
        ActionResult::DirectoryListing { root, entries } => format!(
            "listed {} entr{} under {} [{}]",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
            root.display(),
            preview_paths(entries.iter().map(|entry| entry.path.clone()).collect())
        ),
        ActionResult::FileMatches {
            pattern, matches, ..
        } => format!(
            "found {} match{} for {} [{}]",
            matches.len(),
            if matches.len() == 1 { "" } else { "es" },
            pattern,
            preview_paths(matches.clone())
        ),
        ActionResult::FileRead {
            path,
            content,
            truncated,
        } => format!(
            "read {} ({} chars{}): {}",
            path.display(),
            content.chars().count(),
            if *truncated { ", truncated" } else { "" },
            preview_text(content, 120)
        ),
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            format,
        } => format!(
            "extracted {} text from {} ({} chars{}): {}",
            format,
            path.display(),
            content.chars().count(),
            if *truncated { ", truncated" } else { "" },
            preview_text(content, 120)
        ),
        ActionResult::TextSearch { query, matches, .. } => format!(
            "found {} text match{} for {} [{}]",
            matches.len(),
            if matches.len() == 1 { "" } else { "es" },
            query,
            preview_search_matches(matches)
        ),
        ActionResult::FileWrite {
            path,
            bytes_written,
            appended,
        } => format!(
            "{} {} ({} bytes)",
            if *appended { "appended to" } else { "wrote" },
            path.display(),
            bytes_written
        ),
        ActionResult::NoteRecorded { note } => format!("recorded note: {}", note),
        ActionResult::Response { message } => format!("responded: {}", message),
    }
}

fn compact_action_result_for_context(result: &ActionResult) -> serde_json::Result<String> {
    let compact = match result {
        ActionResult::Command(command) => serde_json::json!({
            "type": "command",
            "command": command.command,
            "cwd": command.cwd,
            "success": command.success,
            "exit_code": command.exit_code,
            "cancelled": command.cancelled,
            "termination": command.termination,
            "stdout": preview_text(&command.stdout, 2000),
            "stderr": preview_text(&command.stderr, 1000),
        }),
        ActionResult::Inspection(world) => serde_json::json!({
            "type": "inspection",
            "cwd": world.cwd,
            "paths": world
                .files
                .iter()
                .take(8)
                .map(|path| path.path.display().to_string())
                .collect::<Vec<_>>(),
        }),
        ActionResult::DirectoryListing { root, entries } => serde_json::json!({
            "type": "directory_listing",
            "root": root,
            "count": entries.len(),
            "entries": entries
                .iter()
                .take(12)
                .map(|entry| serde_json::json!({
                    "path": entry.path,
                    "is_dir": entry.is_dir
                }))
                .collect::<Vec<_>>(),
        }),
        ActionResult::FileMatches {
            root,
            pattern,
            matches,
        } => serde_json::json!({
            "type": "file_matches",
            "root": root,
            "pattern": pattern,
            "count": matches.len(),
            "matches": matches.iter().take(12).collect::<Vec<_>>(),
        }),
        ActionResult::FileRead {
            path,
            content,
            truncated,
        } => serde_json::json!({
            "type": "file_read",
            "path": path,
            "truncated": truncated,
            "content": preview_text(content, 8000),
        }),
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            format,
        } => serde_json::json!({
            "type": "document_text",
            "path": path,
            "format": format,
            "truncated": truncated,
            "content": preview_text(content, 8000),
        }),
        ActionResult::TextSearch {
            root,
            query,
            matches,
        } => serde_json::json!({
            "type": "text_search",
            "root": root,
            "query": query,
            "count": matches.len(),
            "matches": matches
                .iter()
                .take(8)
                .map(|item| serde_json::json!({
                    "path": item.path,
                    "line_number": item.line_number,
                    "line": preview_text(&item.line, 180),
                }))
                .collect::<Vec<_>>(),
        }),
        ActionResult::FileWrite {
            path,
            bytes_written,
            appended,
        } => serde_json::json!({
            "type": "file_write",
            "path": path,
            "bytes_written": bytes_written,
            "appended": appended,
        }),
        ActionResult::NoteRecorded { note } => serde_json::json!({
            "type": "note",
            "note": preview_text(note, 200),
        }),
        ActionResult::Response { message } => serde_json::json!({
            "type": "response",
            "message": preview_text(message, 200),
        }),
    };
    serde_json::to_string(&compact)
}

fn preview_paths(paths: Vec<std::path::PathBuf>) -> String {
    let preview = paths
        .into_iter()
        .take(3)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "no examples".to_string();
    }
    preview.join(", ")
}

fn preview_search_matches(matches: &[SearchMatch]) -> String {
    let preview = matches
        .iter()
        .take(3)
        .map(|item| {
            format!(
                "{}:{} {}",
                item.path.display(),
                item.line_number,
                preview_text(&item.line, 60)
            )
        })
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "no examples".to_string();
    }
    preview.join(" | ")
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn repeated_step_signature(action: &Action, result: &ActionResult) -> Option<String> {
    if matches!(action, Action::Respond { .. } | Action::RecordNote { .. }) {
        return None;
    }

    Some(format!(
        "{}::{}",
        action_label(action),
        summarize_action_result(result)
    ))
}

fn trim_recent_steps(recent_steps: &mut Vec<String>) {
    const MAX_RECENT_STEPS: usize = 6;
    if recent_steps.len() > MAX_RECENT_STEPS {
        let excess = recent_steps.len() - MAX_RECENT_STEPS;
        recent_steps.drain(0..excess);
    }
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
        Action::ExtractDocumentText { path, .. } => {
            format!("extract_document_text:{}", path.display())
        }
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
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct GuidanceReasoner {
        seen_guidance: Arc<Mutex<Vec<Option<String>>>>,
        responses: Arc<Mutex<Vec<ReasonResponse>>>,
    }

    impl GuidanceReasoner {
        fn new(responses: Vec<ReasonResponse>) -> Self {
            Self {
                seen_guidance: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn seen_guidance(&self) -> Vec<Option<String>> {
            self.seen_guidance.lock().unwrap().clone()
        }
    }

    impl retina_traits::Reasoner for GuidanceReasoner {
        fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
            self.seen_guidance
                .lock()
                .unwrap()
                .push(request.context.operator_guidance.clone());
            let mut responses = self.responses.lock().unwrap();
            Ok(if responses.len() > 1 {
                responses.remove(0)
            } else {
                responses.first().cloned().unwrap()
            })
        }

        fn capabilities(&self) -> ReasonerCapabilities {
            ReasonerCapabilities {
                max_context_tokens: 1_000,
                supports_tool_use: false,
                supports_vision: false,
                supports_caching: false,
                model_id: "guidance-test".to_string(),
            }
        }
    }

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
        let _ = kernel
            .execute_task(Task::new(AgentId::new(), task))
            .unwrap();

        assert!(memory.rule_count() >= 1);
    }

    #[test]
    fn successful_read_steps_get_positive_utility() {
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

        let _ = kernel
            .execute_task(Task::new(AgentId::new(), "read startup.md"))
            .unwrap();

        let experiences = memory.experiences();
        assert_eq!(experiences.len(), 1);
        assert!(experiences[0].utility > 0.0);
    }

    #[test]
    fn multi_step_task_continues_until_terminal_step() {
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::FindFiles {
                        id: ActionId::new(),
                        root: ".".into(),
                        pattern: "startup.md".to_string(),
                        max_results: 5,
                    },
                    task_complete: false,
                    reasoning: Some("find it first".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "startup.md".into(),
                        max_bytes: None,
                    },
                    task_complete: true,
                    reasoning: Some("now read it".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        )
        .unwrap();

        let outcome = kernel
            .execute_task(Task::new(AgentId::new(), "find startup.md and read it"))
            .unwrap();
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::FileRead { .. })
        ));
    }

    #[test]
    fn repeated_identical_step_without_progress_fails_honestly() {
        let repeated_action = Action::FindFiles {
            id: ActionId::new(),
            root: ".".into(),
            pattern: "startup.md".to_string(),
            max_results: 5,
        };
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: repeated_action.clone(),
                    task_complete: false,
                    reasoning: Some("find it first".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: repeated_action.clone(),
                    task_complete: false,
                    reasoning: Some("trying the same thing again".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        )
        .unwrap();

        let outcome = kernel
            .execute_task(Task::new(AgentId::new(), "find startup.md and read it"))
            .unwrap();
        assert!(matches!(
            outcome,
            Outcome::Failure(reason) if reason.contains("repeated the same step")
        ));
    }

    #[test]
    fn interactive_stop_cancels_continuation() {
        let control = ExecutionControl::new();
        control.handle().request_cancel();
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_response(ReasonResponse {
                action: Action::FindFiles {
                    id: ActionId::new(),
                    root: ".".into(),
                    pattern: "startup.md".to_string(),
                    max_results: 5,
                },
                task_complete: false,
                reasoning: Some("find it first".to_string()),
                tokens_used: TokenUsage::default(),
            })),
            Box::new(MockMemory::default()),
        )
        .unwrap();

        let outcome = kernel
            .execute_task_with_config(
                Task::new(AgentId::new(), "find startup.md and read it"),
                ExecutionConfig {
                    max_steps: 3,
                    control: Some(control.handle()),
                },
            )
            .unwrap();
        assert!(matches!(outcome, Outcome::Blocked(reason) if reason.contains("cancelled")));
    }

    #[test]
    fn guidance_is_applied_once_to_the_next_planning_step() {
        let control = ExecutionControl::new();
        let handle = control.handle();
        handle.queue_guidance("prefer the markdown file");
        let reasoner = GuidanceReasoner::new(vec![
            ReasonResponse {
                action: Action::FindFiles {
                    id: ActionId::new(),
                    root: ".".into(),
                    pattern: "startup.md".to_string(),
                    max_results: 5,
                },
                task_complete: false,
                reasoning: Some("find it first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                reasoning: Some("respond".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ]);
        let inspector = reasoner.clone();
        let kernel = Kernel::new(
            Box::new(MockShell::default()),
            Box::new(reasoner),
            Box::new(MockMemory::default()),
        )
        .unwrap();

        let outcome = kernel
            .execute_task_with_config(
                Task::new(AgentId::new(), "find startup.md and answer"),
                ExecutionConfig {
                    max_steps: 3,
                    control: Some(handle),
                },
            )
            .unwrap();
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));
        let seen = inspector.seen_guidance();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].as_deref(), Some("prefer the markdown file"));
        assert_eq!(seen[1], None);
    }
}
