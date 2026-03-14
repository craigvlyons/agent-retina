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
                "network_enabled": routing.network_enabled,
                "task_state": self.build_task_state(
                    &task,
                    &TaskLoopState::new(config.max_steps),
                    1,
                    config.max_steps,
                    None,
                )
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
                StepSelectionContext {
                    task: &task,
                    intent: &intent,
                    state: &state,
                    control: config.control.as_ref(),
                    current_step: state.step_index + 1,
                    max_steps: config.max_steps,
                },
                next_reflex_action.take(),
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
            let compaction = state.apply_live_compaction();
            let task_state = self.build_task_state(
                &task,
                &state,
                state.step_index.max(1),
                config.max_steps,
                state.last_result_summary.clone(),
            );

            if let Some(compaction) = compaction {
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCompacted,
                    json!({
                        "reason": compaction.reason,
                        "score_explanations": compaction.score_explanations,
                        "task_state": task_state.clone()
                    }),
                ))?;
            }

            self.emit_event(EventSpec::new(
                &task,
                Some(&intent),
                Some(&step.action),
                TimelineEventType::TaskStepCompleted,
                json!({
                    "result": "step_completed",
                    "task_state": task_state
                }),
            ))?;

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

            if step.task_complete {
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCompleted,
                    json!({
                        "outcome": "success",
                        "task_state": task_state
                    }),
                ))?;
            }

            if step.task_complete || matches!(outcome, Outcome::Failure(_) | Outcome::Blocked(_)) {
                return Ok(outcome);
            }
        }
    }

    fn select_action(
        &self,
        selection: StepSelectionContext<'_>,
        reflex_action: Option<Action>,
    ) -> Result<StepDecision> {
        let StepSelectionContext {
            task,
            intent,
            state,
            control,
            current_step,
            max_steps,
        } = selection;
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

        let context = self.assemble_context(ContextAssemblyInput {
            task,
            state,
            last_result: state.last_result_json.clone(),
            last_result_summary: state.last_result_summary.clone(),
            operator_guidance: control.and_then(ExecutionControlHandle::take_guidance),
            current_step,
            max_steps,
        })?;
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
                return self.cancel_outcome(
                    task,
                    Some(intent),
                    Some(&action),
                    "task cancelled by operator during approval",
                );
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
                return self.cancel_outcome(
                    task,
                    Some(intent),
                    Some(&action),
                    command
                        .termination
                        .clone()
                        .unwrap_or_else(|| "task cancelled by operator".to_string()),
                );
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

        let mut reflection_state = TaskLoopState::new(1);
        reflection_state.recent_steps = vec![format!("failed action: {}", action_label(action))];
        reflection_state.recent_action_summaries = vec![RecentActionSummary {
            step: 1,
            action: action_label(action),
            outcome: reason.clone(),
            artifact_refs: Vec::new(),
        }];
        reflection_state.avoid_rules = vec![AvoidRule {
            label: action_label(action),
            reason: reason.clone(),
        }];
        let reflection_context = self.assemble_context(ContextAssemblyInput {
            task,
            state: &reflection_state,
            last_result: Some(reason.clone()),
            last_result_summary: Some(reason.clone()),
            operator_guidance: control.and_then(ExecutionControlHandle::take_guidance),
            current_step: 1,
            max_steps: 1,
        })?;
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

    fn assemble_context(&self, input: ContextAssemblyInput<'_>) -> Result<AssembledContext> {
        let ContextAssemblyInput {
            task,
            state,
            last_result,
            last_result_summary,
            operator_guidance,
            current_step,
            max_steps,
        } = input;
        let shell_constraints = self
            .shell
            .constraints()
            .iter()
            .map(|constraint| format!("{constraint:?}"))
            .collect::<Vec<_>>();
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
            task_state: self
                .build_task_state(
                    task,
                    state,
                    current_step,
                    max_steps,
                    last_result_summary.clone(),
                )
                .with_constraints(shell_constraints),
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
            recent_steps: state.recent_steps.clone(),
            operator_guidance,
            current_step,
            max_steps,
        })
    }

    fn build_task_state(
        &self,
        task: &Task,
        state: &TaskLoopState,
        current_step: usize,
        max_steps: usize,
        last_result_summary: Option<String>,
    ) -> TaskState {
        TaskState {
            goal: TaskGoal {
                objective: task.description.clone(),
                success_criteria: Vec::new(),
                constraints: Vec::new(),
            },
            progress: TaskProgress {
                current_phase: describe_task_phase(state, current_step, max_steps),
                current_step,
                max_steps,
                completed_checkpoints: state.recent_steps.clone(),
                verified_facts: summarize_verified_facts(&state.artifact_references),
            },
            frontier: TaskFrontier {
                next_action_hint: Some(
                    match (last_result_summary, state.last_compaction_reason.as_ref()) {
                        (Some(summary), Some(reason)) => format!(
                            "Continue from compact task state ({reason}); use the latest verified result to choose the next smallest useful step: {summary}"
                        ),
                        (Some(summary), None) => format!(
                            "Use the latest verified result to choose the next smallest useful step: {summary}"
                        ),
                        (None, Some(reason)) => {
                            format!("Continue from compact task state ({reason})")
                        }
                        (None, None) => {
                            "Choose the next smallest useful step from current task state"
                                .to_string()
                        }
                    },
                ),
                open_questions: Vec::new(),
                blockers: Vec::new(),
            },
            recent_actions: state.recent_action_summaries.clone(),
            working_sources: state.working_sources.clone(),
            artifact_references: state.artifact_references.clone(),
            avoid: state.avoid_rules.clone(),
            compaction: state.last_compaction_snapshot.clone(),
        }
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

struct StepSelectionContext<'a> {
    task: &'a Task,
    intent: &'a Intent,
    state: &'a TaskLoopState,
    control: Option<&'a ExecutionControlHandle>,
    current_step: usize,
    max_steps: usize,
}

struct ContextAssemblyInput<'a> {
    task: &'a Task,
    state: &'a TaskLoopState,
    last_result: Option<String>,
    last_result_summary: Option<String>,
    operator_guidance: Option<String>,
    current_step: usize,
    max_steps: usize,
}

struct TaskLoopState {
    step_index: usize,
    last_result_json: Option<String>,
    last_result_summary: Option<String>,
    recent_steps: Vec<String>,
    recent_action_summaries: Vec<RecentActionSummary>,
    working_sources: Vec<WorkingSource>,
    artifact_references: Vec<ArtifactReference>,
    avoid_rules: Vec<AvoidRule>,
    compaction_count: usize,
    last_compaction_reason: Option<String>,
    last_compaction_snapshot: Option<CompactionSnapshot>,
    seen_signatures: HashMap<String, usize>,
}

impl TaskLoopState {
    fn new(_max_steps: usize) -> Self {
        Self {
            step_index: 0,
            last_result_json: None,
            last_result_summary: None,
            recent_steps: Vec::new(),
            recent_action_summaries: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            avoid_rules: Vec::new(),
            compaction_count: 0,
            last_compaction_reason: None,
            last_compaction_snapshot: None,
            seen_signatures: HashMap::new(),
        }
    }

    fn record_step(&mut self, step: &StepDecision, outcome: &Outcome) -> Result<StepProgress> {
        self.step_index += 1;
        let mut repeated_without_progress = false;
        self.last_result_json = match outcome {
            Outcome::Success(result) if !matches!(step.action, Action::Respond { .. }) => {
                let summary = summarize_action_result(result);
                let artifact_refs = artifact_references_for_result(result);
                let working_sources =
                    working_sources_for_result(&step.action, result, self.step_index + 1);
                self.last_result_summary = Some(summary.clone());
                self.recent_steps.push(format!(
                    "step {}: {} -> {}",
                    self.step_index,
                    action_label(&step.action),
                    summary
                ));
                trim_recent_steps(&mut self.recent_steps);
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(&step.action),
                    outcome: summary.clone(),
                    artifact_refs: artifact_refs.clone(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
                merge_working_sources(&mut self.working_sources, working_sources);
                merge_artifact_references(&mut self.artifact_references, artifact_refs);
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
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(&step.action),
                    outcome: "responded to operator".to_string(),
                    artifact_refs: Vec::new(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
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
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(&step.action),
                    outcome: reason.clone(),
                    artifact_refs: Vec::new(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
                self.avoid_rules.push(AvoidRule {
                    label: action_label(&step.action),
                    reason: reason.clone(),
                });
                trim_avoid_rules(&mut self.avoid_rules);
                None
            }
        };
        Ok(StepProgress {
            repeated_without_progress,
        })
    }

    fn apply_live_compaction(&mut self) -> Option<CompactionDecision> {
        let mut reasons = Vec::new();

        if self.step_index >= 3 && self.recent_steps.len() > 3 {
            reasons.push("step threshold".to_string());
        }
        if self
            .last_result_json
            .as_ref()
            .map(|value| value.len() > 1400)
            .unwrap_or(false)
        {
            reasons.push("large tool result".to_string());
        }
        if self.working_sources.len() > 6 {
            reasons.push("source set growth".to_string());
        }

        if reasons.is_empty() {
            return None;
        }

        let reason = reasons.join(", ");
        let score_explanations = build_compaction_score_explanations(self);
        self.compaction_count += 1;
        self.last_compaction_reason = Some(reason.clone());
        self.last_compaction_snapshot = Some(CompactionSnapshot {
            reason: reason.clone(),
            score_explanations: score_explanations.clone(),
        });

        if let Some(last_result) = self.last_result_json.as_ref() {
            self.last_result_json = compact_last_result_for_compacted_context(last_result).ok();
        }

        trim_recent_steps_for_compacted_state(&mut self.recent_steps);
        trim_recent_action_summaries_for_compacted_state(&mut self.recent_action_summaries);
        trim_working_sources_for_compacted_state(&mut self.working_sources);
        trim_artifact_references_for_compacted_state(&mut self.artifact_references);

        Some(CompactionDecision {
            reason,
            score_explanations,
        })
    }
}

#[derive(Default)]
struct StepProgress {
    repeated_without_progress: bool,
}

struct CompactionDecision {
    reason: String,
    score_explanations: Vec<CompactionScoreExplanation>,
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
    matches!(
        action,
        Action::RunCommand {
            require_approval: true,
            ..
        } | Action::WriteFile {
            require_approval: true,
            ..
        } | Action::AppendFile {
            require_approval: true,
            ..
        }
    )
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

fn trim_recent_action_summaries(recent_actions: &mut Vec<RecentActionSummary>) {
    const MAX_RECENT_ACTION_SUMMARIES: usize = 6;
    if recent_actions.len() > MAX_RECENT_ACTION_SUMMARIES {
        let excess = recent_actions.len() - MAX_RECENT_ACTION_SUMMARIES;
        recent_actions.drain(0..excess);
    }
}

fn trim_avoid_rules(avoid_rules: &mut Vec<AvoidRule>) {
    const MAX_AVOID_RULES: usize = 6;
    if avoid_rules.len() > MAX_AVOID_RULES {
        let excess = avoid_rules.len() - MAX_AVOID_RULES;
        avoid_rules.drain(0..excess);
    }
}

fn merge_artifact_references(
    existing: &mut Vec<ArtifactReference>,
    candidates: Vec<ArtifactReference>,
) {
    const MAX_ARTIFACT_REFERENCES: usize = 12;

    for candidate in candidates {
        if let Some(position) = existing
            .iter()
            .position(|item| item.locator == candidate.locator && item.kind == candidate.kind)
        {
            existing[position] = candidate;
        } else {
            existing.push(candidate);
        }
    }

    if existing.len() > MAX_ARTIFACT_REFERENCES {
        let excess = existing.len() - MAX_ARTIFACT_REFERENCES;
        existing.drain(0..excess);
    }
}

fn merge_working_sources(existing: &mut Vec<WorkingSource>, candidates: Vec<WorkingSource>) {
    const MAX_WORKING_SOURCES: usize = 12;

    for candidate in candidates {
        if let Some(position) = existing
            .iter()
            .position(|item| item.locator == candidate.locator && item.kind == candidate.kind)
        {
            existing[position] = candidate;
        } else {
            existing.push(candidate);
        }
    }

    if existing.len() > MAX_WORKING_SOURCES {
        let excess = existing.len() - MAX_WORKING_SOURCES;
        existing.drain(0..excess);
    }
}

fn trim_recent_steps_for_compacted_state(recent_steps: &mut Vec<String>) {
    const MAX_COMPACTED_RECENT_STEPS: usize = 3;
    if recent_steps.len() > MAX_COMPACTED_RECENT_STEPS {
        let excess = recent_steps.len() - MAX_COMPACTED_RECENT_STEPS;
        recent_steps.drain(0..excess);
    }
}

fn trim_recent_action_summaries_for_compacted_state(recent_actions: &mut Vec<RecentActionSummary>) {
    const MAX_COMPACTED_RECENT_ACTIONS: usize = 3;
    if recent_actions.len() > MAX_COMPACTED_RECENT_ACTIONS {
        let excess = recent_actions.len() - MAX_COMPACTED_RECENT_ACTIONS;
        recent_actions.drain(0..excess);
    }
}

fn trim_working_sources_for_compacted_state(working_sources: &mut Vec<WorkingSource>) {
    const MAX_COMPACTED_WORKING_SOURCES: usize = 6;
    if working_sources.len() > MAX_COMPACTED_WORKING_SOURCES {
        let excess = working_sources.len() - MAX_COMPACTED_WORKING_SOURCES;
        working_sources.drain(0..excess);
    }
}

fn trim_artifact_references_for_compacted_state(artifact_refs: &mut Vec<ArtifactReference>) {
    const MAX_COMPACTED_ARTIFACT_REFS: usize = 8;
    if artifact_refs.len() > MAX_COMPACTED_ARTIFACT_REFS {
        let excess = artifact_refs.len() - MAX_COMPACTED_ARTIFACT_REFS;
        artifact_refs.drain(0..excess);
    }
}

fn compact_last_result_for_compacted_context(last_result: &str) -> serde_json::Result<String> {
    let value: serde_json::Value = serde_json::from_str(last_result)?;
    let compact = match value.get("type").and_then(serde_json::Value::as_str) {
        Some("file_read") => serde_json::json!({
            "type": "file_read",
            "path": value.get("path"),
            "truncated": value.get("truncated"),
            "content_preview": value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(|text| preview_text(text, 240)),
            "continuation": "reopen file by path from task_state artifact refs if more detail is needed"
        }),
        Some("document_text") => serde_json::json!({
            "type": "document_text",
            "path": value.get("path"),
            "format": value.get("format"),
            "truncated": value.get("truncated"),
            "content_preview": value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(|text| preview_text(text, 240)),
            "continuation": "reopen extracted document by path from task_state artifact refs if more detail is needed"
        }),
        Some("text_search") => serde_json::json!({
            "type": "text_search",
            "root": value.get("root"),
            "query": value.get("query"),
            "count": value.get("count"),
            "matches": value.get("matches"),
            "continuation": "use task_state working sources and artifact refs for exact evidence"
        }),
        Some("file_matches") => serde_json::json!({
            "type": "file_matches",
            "root": value.get("root"),
            "pattern": value.get("pattern"),
            "count": value.get("count"),
            "matches": value.get("matches"),
            "continuation": "choose from task_state candidate sources instead of re-searching"
        }),
        Some("directory_listing") => serde_json::json!({
            "type": "directory_listing",
            "root": value.get("root"),
            "count": value.get("count"),
            "entries": value.get("entries"),
            "continuation": "use task_state candidate sources instead of replaying the full listing"
        }),
        _ => value,
    };
    serde_json::to_string(&compact)
}

fn build_compaction_score_explanations(state: &TaskLoopState) -> Vec<CompactionScoreExplanation> {
    let mut explanations = Vec::new();

    for source in &state.working_sources {
        let decision = if source.role == "authoritative" || source.role == "generated" {
            "keep"
        } else if source.status == "matched" || source.status == "listed" {
            "compact"
        } else {
            "keep"
        };
        let rationale = if source.role == "authoritative" {
            "high state dependency and recovery value".to_string()
        } else if source.role == "generated" {
            "captures produced artifact and recovery anchor".to_string()
        } else if source.status == "matched" || source.status == "listed" {
            "useful candidate context but lower forward utility than authoritative sources"
                .to_string()
        } else {
            "still relevant to current frontier".to_string()
        };
        explanations.push(CompactionScoreExplanation {
            item_kind: "source".to_string(),
            locator: source.locator.clone(),
            decision: decision.to_string(),
            rationale,
        });
    }

    for artifact in &state.artifact_references {
        explanations.push(CompactionScoreExplanation {
            item_kind: "artifact".to_string(),
            locator: artifact.locator.clone(),
            decision: "keep_ref".to_string(),
            rationale: "exact evidence reference preserved for recovery and re-open".to_string(),
        });
    }

    for avoid in &state.avoid_rules {
        explanations.push(CompactionScoreExplanation {
            item_kind: "avoid".to_string(),
            locator: avoid.label.clone(),
            decision: "keep".to_string(),
            rationale: "failed path preserved to avoid repeating harmful work".to_string(),
        });
    }

    explanations
}

fn describe_task_phase(state: &TaskLoopState, current_step: usize, max_steps: usize) -> String {
    if state.step_index == 0 {
        "starting".to_string()
    } else if current_step >= max_steps {
        "final step".to_string()
    } else {
        format!("working through step {} of {}", current_step, max_steps)
    }
}

fn summarize_verified_facts(references: &[ArtifactReference]) -> Vec<String> {
    references
        .iter()
        .take(8)
        .map(|reference| match reference.status.as_str() {
            "read" => format!("read {}", reference.locator),
            "extracted" => format!("extracted text from {}", reference.locator),
            "matched" => format!("matched candidate {}", reference.locator),
            "searched" => format!("searched and found evidence in {}", reference.locator),
            "listed" => format!("listed directory {}", reference.locator),
            "written" => format!("wrote {}", reference.locator),
            "appended" => format!("appended {}", reference.locator),
            _ => format!("{} {}", reference.status, reference.locator),
        })
        .collect()
}

fn artifact_references_for_result(result: &ActionResult) -> Vec<ArtifactReference> {
    match result {
        ActionResult::Command(command) => vec![ArtifactReference {
            kind: "command".to_string(),
            locator: command.command.clone(),
            status: if command.cancelled {
                "cancelled".to_string()
            } else if command.success {
                "executed".to_string()
            } else {
                "failed".to_string()
            },
        }],
        ActionResult::Inspection(world) => world
            .files
            .iter()
            .take(6)
            .map(|item| ArtifactReference {
                kind: "path".to_string(),
                locator: item.path.display().to_string(),
                status: "inspected".to_string(),
            })
            .collect(),
        ActionResult::DirectoryListing { root, .. } => vec![ArtifactReference {
            kind: "directory".to_string(),
            locator: root.display().to_string(),
            status: "listed".to_string(),
        }],
        ActionResult::FileMatches { matches, .. } => matches
            .iter()
            .take(6)
            .map(|path| ArtifactReference {
                kind: if path.is_dir() { "directory" } else { "file" }.to_string(),
                locator: path.display().to_string(),
                status: "matched".to_string(),
            })
            .collect(),
        ActionResult::FileRead { path, .. } => vec![ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: "read".to_string(),
        }],
        ActionResult::DocumentText { path, .. } => vec![ArtifactReference {
            kind: "document".to_string(),
            locator: path.display().to_string(),
            status: "extracted".to_string(),
        }],
        ActionResult::TextSearch { matches, .. } => matches
            .iter()
            .take(6)
            .map(|item| ArtifactReference {
                kind: "file".to_string(),
                locator: item.path.display().to_string(),
                status: "searched".to_string(),
            })
            .collect(),
        ActionResult::FileWrite { path, appended, .. } => vec![ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: if *appended {
                "appended".to_string()
            } else {
                "written".to_string()
            },
        }],
        ActionResult::NoteRecorded { .. } | ActionResult::Response { .. } => Vec::new(),
    }
}

fn working_sources_for_result(
    action: &Action,
    result: &ActionResult,
    step_index: usize,
) -> Vec<WorkingSource> {
    match result {
        ActionResult::Inspection(world) => world
            .files
            .iter()
            .take(6)
            .map(|item| WorkingSource {
                kind: "path".to_string(),
                locator: item.path.display().to_string(),
                role: "supporting".to_string(),
                status: "inspected".to_string(),
                why_it_matters: format!("observed while {}", action_label(action)),
                last_used_step: step_index,
                evidence_refs: vec![item.path.display().to_string()],
            })
            .collect(),
        ActionResult::DirectoryListing { root, .. } => vec![WorkingSource {
            kind: "directory".to_string(),
            locator: root.display().to_string(),
            role: "supporting".to_string(),
            status: "listed".to_string(),
            why_it_matters: "directory explored for task-relevant candidates".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![root.display().to_string()],
        }],
        ActionResult::FileMatches { matches, .. } => matches
            .iter()
            .take(6)
            .map(|path| WorkingSource {
                kind: if path.is_dir() { "directory" } else { "file" }.to_string(),
                locator: path.display().to_string(),
                role: "candidate".to_string(),
                status: "matched".to_string(),
                why_it_matters: "candidate source discovered for the task".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![path.display().to_string()],
            })
            .collect(),
        ActionResult::FileRead { path, .. } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "content source currently informing the task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
        }],
        ActionResult::DocumentText { path, .. } => vec![WorkingSource {
            kind: "document".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "excerpted".to_string(),
            why_it_matters: "document text extracted for task reasoning".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
        }],
        ActionResult::TextSearch { matches, .. } => matches
            .iter()
            .take(6)
            .map(|item| WorkingSource {
                kind: "file".to_string(),
                locator: item.path.display().to_string(),
                role: "supporting".to_string(),
                status: "matched_text".to_string(),
                why_it_matters: "contains text evidence relevant to the task".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![format!("{}:{}", item.path.display(), item.line_number)],
            })
            .collect(),
        ActionResult::FileWrite { path, appended, .. } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "generated".to_string(),
            status: if *appended {
                "appended".to_string()
            } else {
                "written".to_string()
            },
            why_it_matters: "task produced or updated this artifact".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
        }],
        ActionResult::Command(command) => vec![WorkingSource {
            kind: "command".to_string(),
            locator: command.command.clone(),
            role: "supporting".to_string(),
            status: if command.cancelled {
                "cancelled".to_string()
            } else if command.success {
                "executed".to_string()
            } else {
                "failed".to_string()
            },
            why_it_matters: "shell command executed as part of the task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![command.command.clone()],
        }],
        ActionResult::NoteRecorded { note } => vec![WorkingSource {
            kind: "note".to_string(),
            locator: preview_text(note, 80),
            role: "generated".to_string(),
            status: "recorded".to_string(),
            why_it_matters: "operator/task note captured by the harness".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![preview_text(note, 80)],
        }],
        ActionResult::Response { .. } => Vec::new(),
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

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
    }

    fn must_some<T>(value: Option<T>, message: &str) -> T {
        value.unwrap_or_else(|| panic!("{message}"))
    }
    use retina_test_utils::{MockMemory, MockReasoner, MockShell};
    use retina_traits::Shell;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct GuidanceReasoner {
        seen_guidance: Arc<Mutex<Vec<Option<String>>>>,
        seen_task_states: Arc<Mutex<Vec<TaskState>>>,
        responses: Arc<Mutex<Vec<ReasonResponse>>>,
    }

    impl GuidanceReasoner {
        fn new(responses: Vec<ReasonResponse>) -> Self {
            Self {
                seen_guidance: Arc::new(Mutex::new(Vec::new())),
                seen_task_states: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn seen_guidance(&self) -> Vec<Option<String>> {
            recover_mutex(&self.seen_guidance).clone()
        }

        fn seen_task_states(&self) -> Vec<TaskState> {
            recover_mutex(&self.seen_task_states).clone()
        }
    }

    impl retina_traits::Reasoner for GuidanceReasoner {
        fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
            recover_mutex(&self.seen_guidance).push(request.context.operator_guidance.clone());
            recover_mutex(&self.seen_task_states).push(request.context.task_state.clone());
            let mut responses = recover_mutex(&self.responses);
            Ok(if responses.len() > 1 {
                responses.remove(0)
            } else {
                responses
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ReasonResponse {
                        action: Action::Respond {
                            id: ActionId::new(),
                            message: "done".to_string(),
                        },
                        task_complete: true,
                        reasoning: Some("fallback test response".to_string()),
                        tokens_used: TokenUsage::default(),
                    })
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

    #[derive(Default)]
    struct LargeReadShell;

    impl Shell for LargeReadShell {
        fn observe(&self) -> Result<WorldState> {
            Ok(WorldState {
                cwd: PathBuf::from("."),
                files: Vec::new(),
                last_command: None,
                notes: Vec::new(),
            })
        }

        fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
            Ok(StateSnapshot {
                scope: scope.clone(),
                cwd: PathBuf::from("."),
                cwd_hash: "stable".to_string(),
                files: Vec::new(),
                last_command: None,
            })
        }

        fn compare_state(
            &self,
            _before: &StateSnapshot,
            _after: &StateSnapshot,
            _action: Option<&Action>,
        ) -> Result<StateDelta> {
            Ok(StateDelta {
                kind: StateDeltaKind::ChangedAsExpected,
                summary: "changed".to_string(),
                changed_paths: Vec::new(),
            })
        }

        fn execute(&self, action: &Action) -> Result<ActionResult> {
            match action {
                Action::ReadFile { path, .. } => Ok(ActionResult::FileRead {
                    path: path.clone(),
                    content: "a".repeat(4000),
                    truncated: false,
                }),
                Action::Respond { message, .. } => Ok(ActionResult::Response {
                    message: message.clone(),
                }),
                _ => Err(KernelError::Unsupported(
                    "unsupported test action".to_string(),
                )),
            }
        }

        fn constraints(&self) -> &[HardConstraint] {
            const CONSTRAINTS: &[HardConstraint] = &[HardConstraint::NoNetworkShellActions];
            CONSTRAINTS
        }

        fn capabilities(&self) -> ShellCapabilities {
            ShellCapabilities {
                can_execute_commands: false,
                can_read_files: true,
                can_write_files: false,
                can_search_files: false,
                can_extract_documents: false,
                can_write_notes: false,
                can_respond_text: true,
            }
        }

        fn request_approval(&self, _request: &ApprovalRequest) -> Result<ApprovalResponse> {
            Ok(ApprovalResponse::Approved)
        }

        fn notify(&self, _message: &str) -> Result<()> {
            Ok(())
        }

        fn request_input(&self, _prompt: &str) -> Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn router_defaults_to_handle_directly() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::Respond {
                id: ActionId::new(),
                message: "hello".to_string(),
            })),
            Box::new(MockMemory::default()),
        ));
        let task = Task::new(AgentId::new(), "inspect");
        assert!(matches!(
            kernel.route_task(&task),
            RoutingDecision::HandleDirectly
        ));
    }

    #[test]
    fn assembled_context_includes_structured_task_state() {
        let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            reasoning: Some("test".to_string()),
            tokens_used: TokenUsage::default(),
        }]);
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(reasoner.clone()),
            Box::new(MockMemory::default()),
        ));

        let task = Task::new(AgentId::new(), "read startup.md");
        let outcome = must(kernel.execute_task(task));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));

        let seen = reasoner.seen_task_states();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].goal.objective, "read startup.md");
        assert_eq!(seen[0].progress.current_phase, "starting");
    }

    #[test]
    fn task_step_snapshot_tracks_working_sources() {
        let memory = MockMemory::default();
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::ReadFile {
                id: ActionId::new(),
                path: "startup.md".into(),
                max_bytes: Some(1024),
            })),
            Box::new(memory.clone()),
        ));

        let outcome = must(kernel.execute_task(Task::new(AgentId::new(), "read startup.md")));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::FileRead { .. })
        ));

        let events = must(memory.recent_states(20));
        let task_state = events
            .into_iter()
            .find_map(|event| event.payload_json.get("task_state").cloned())
            .and_then(|value| serde_json::from_value::<TaskState>(value).ok());
        let task_state = must_some(task_state, "expected task_state snapshot");

        assert!(
            task_state
                .working_sources
                .iter()
                .any(|source| source.locator.ends_with("startup.md") && source.status == "read")
        );
    }

    #[test]
    fn large_result_triggers_live_compaction_event() {
        let memory = MockMemory::default();
        let kernel = must(Kernel::new(
            Box::new(LargeReadShell),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "big.md".into(),
                        max_bytes: Some(8000),
                    },
                    task_complete: false,
                    reasoning: Some("read".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::Respond {
                        id: ActionId::new(),
                        message: "done".to_string(),
                    },
                    task_complete: true,
                    reasoning: Some("answer".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(memory.clone()),
        ));

        let outcome = must(kernel.execute_task(Task::new(AgentId::new(), "read big.md")));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));

        let events = must(memory.recent_states(30));
        let compacted = must_some(
            events
                .iter()
                .find(|event| event.event_type == TimelineEventType::TaskCompacted),
            "expected compaction event",
        );
        let reason = compacted
            .payload_json
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(reason.contains("large tool result"));

        let task_state = compacted
            .payload_json
            .get("task_state")
            .cloned()
            .and_then(|value| serde_json::from_value::<TaskState>(value).ok());
        let task_state = must_some(task_state, "expected compacted task state");
        let snapshot = must_some(task_state.compaction, "expected compaction snapshot");
        assert!(!snapshot.score_explanations.is_empty());
        assert!(
            snapshot
                .score_explanations
                .iter()
                .any(|item| item.item_kind == "artifact" && item.decision == "keep_ref")
        );
    }

    #[test]
    fn execute_loop_records_timeline() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::Respond {
                id: ActionId::new(),
                message: "hello".to_string(),
            })),
            Box::new(MockMemory::default()),
        ));
        let task = Task::new(AgentId::new(), "hello");
        let outcome = must(kernel.execute_task(task));
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
        let kernel = must(Kernel::new(
            Box::new(shell),
            Box::new(MockReasoner::for_action(action.clone())),
            Box::new(MockMemory::default()),
        ));
        let task = Task::new(AgentId::new(), "run echo hi > note.txt");
        let outcome = must(kernel.execute_task(task));
        assert!(matches!(outcome, Outcome::Failure(_)));
    }

    #[test]
    fn repeated_successful_pattern_promotes_rule() {
        let memory = MockMemory::default();
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::ReadFile {
                id: ActionId::new(),
                path: "startup.md".into(),
                max_bytes: None,
            })),
            Box::new(memory.clone()),
        ));

        let task = "read startup.md";
        let _ = must(kernel.execute_task(Task::new(AgentId::new(), task)));
        let _ = must(kernel.execute_task(Task::new(AgentId::new(), task)));
        let _ = must(kernel.execute_task(Task::new(AgentId::new(), task)));

        assert!(memory.rule_count() >= 1);
    }

    #[test]
    fn successful_read_steps_get_positive_utility() {
        let memory = MockMemory::default();
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::for_action(Action::ReadFile {
                id: ActionId::new(),
                path: "startup.md".into(),
                max_bytes: None,
            })),
            Box::new(memory.clone()),
        ));

        let _ = must(kernel.execute_task(Task::new(AgentId::new(), "read startup.md")));

        let experiences = memory.experiences();
        assert_eq!(experiences.len(), 1);
        assert!(experiences[0].utility > 0.0);
    }

    #[test]
    fn multi_step_task_continues_until_terminal_step() {
        let kernel = must(Kernel::new(
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
        ));

        let outcome =
            must(kernel.execute_task(Task::new(AgentId::new(), "find startup.md and read it")));
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
        let kernel = must(Kernel::new(
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
        ));

        let outcome =
            must(kernel.execute_task(Task::new(AgentId::new(), "find startup.md and read it")));
        assert!(matches!(
            outcome,
            Outcome::Failure(reason) if reason.contains("repeated the same step")
        ));
    }

    #[test]
    fn interactive_stop_cancels_continuation() {
        let control = ExecutionControl::new();
        control.handle().request_cancel();
        let kernel = must(Kernel::new(
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
        ));

        let outcome = must(kernel.execute_task_with_config(
            Task::new(AgentId::new(), "find startup.md and read it"),
            ExecutionConfig {
                max_steps: 3,
                control: Some(control.handle()),
            },
        ));
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
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(reasoner),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task_with_config(
            Task::new(AgentId::new(), "find startup.md and answer"),
            ExecutionConfig {
                max_steps: 3,
                control: Some(handle),
            },
        ));
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
