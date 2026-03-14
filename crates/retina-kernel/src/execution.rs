use crate::result_helpers::{should_retry, summarize_verified_facts};
use crate::support::{
    ContextAssemblyInput, EventSpec, StepDecision, StepSelectionContext, action_failure_reason,
    action_requires_approval, action_utility, approval_reason, default_tool_descriptors,
};
use crate::task_shape::{
    build_task_frontier, describe_task_phase, infer_task_shape, required_input_is_satisfied,
};
use crate::{Kernel, TaskLoopState, action_label};
use chrono::Utc;
use retina_types::*;
use serde_json::json;

impl Kernel {
    pub(crate) fn select_action(
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

    pub(crate) fn execute_action(
        &self,
        task: &Task,
        intent: &mut Intent,
        step: &StepDecision,
        control: Option<&ExecutionControlHandle>,
        allow_retry: bool,
    ) -> Result<Outcome> {
        let mut action = step.action.clone();
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
            action.mark_approval_granted();
            intent.action = Some(action.clone());
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

    pub(crate) fn reflect_or_fail(
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

    pub(crate) fn record_experience(
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

    pub(crate) fn assemble_context(
        &self,
        input: ContextAssemblyInput<'_>,
    ) -> Result<AssembledContext> {
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

    pub(crate) fn build_task_state(
        &self,
        task: &Task,
        state: &TaskLoopState,
        current_step: usize,
        max_steps: usize,
        last_result_summary: Option<String>,
    ) -> TaskState {
        let shape = infer_task_shape(&task.description, state);
        let (open_questions, blockers, next_action_hint) =
            build_task_frontier(&shape, last_result_summary.clone(), state);
        TaskState {
            goal: TaskGoal {
                objective: task.description.clone(),
                success_criteria: Vec::new(),
                constraints: Vec::new(),
            },
            shape: shape.clone(),
            progress: TaskProgress {
                current_phase: describe_task_phase(state, current_step, max_steps),
                current_step,
                max_steps,
                completed_checkpoints: state.recent_steps.clone(),
                verified_facts: summarize_verified_facts(&state.artifact_references),
                required_inputs: shape.required_inputs.len(),
                satisfied_inputs: shape
                    .required_inputs
                    .iter()
                    .filter(|input| required_input_is_satisfied(input))
                    .count(),
                output_written: shape
                    .requested_output
                    .as_ref()
                    .map(|output| output.exists)
                    .unwrap_or(false),
                output_verified: shape
                    .requested_output
                    .as_ref()
                    .map(|output| output.verified)
                    .unwrap_or(false),
            },
            frontier: TaskFrontier {
                next_action_hint,
                open_questions,
                blockers,
            },
            recent_actions: state.recent_action_summaries.clone(),
            working_sources: state.working_sources.clone(),
            artifact_references: state.artifact_references.clone(),
            avoid: state.avoid_rules.clone(),
            compaction: state.last_compaction_snapshot.clone(),
        }
    }

    pub(crate) fn check_cancellation(
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

    pub(crate) fn cancel_outcome(
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

    pub(crate) fn emit_event(&self, spec: EventSpec<'_>) -> Result<()> {
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
