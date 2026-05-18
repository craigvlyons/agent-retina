// File boundary: keep lib.rs focused on kernel orchestration and top-level
// module wiring. Move new helpers, policies, and feature logic into modules.
mod execution;
mod loop_state;
mod result_helpers;
mod router;
mod support;

pub(crate) use crate::loop_state::{TaskLoopState, action_label};
use crate::router::Router;
pub(crate) use crate::support::{
    ActionExecution, EventSpec, StepSelectionContext, completion_blocker_reason,
    select_reflex_action, tool_authored_completion_message,
};
use retina_tools::ToolPolicy;
use retina_traits::{AgentRuntime, McpRuntime, Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::sync::Arc;

const MAX_OUTPUT_TOKENS_RECOVERY_LIMIT: usize = 3;
const MAX_OUTPUT_TOKENS_RECOVERY_SIGNATURE: &str = "max_output_tokens_recovery";
const MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE: &str = "Output token limit hit. Resume directly with no apology or recap. Pick up mid-thought if needed. Break the remaining work into smaller pieces.";
const PROMPT_TOO_LONG_COMPACTION_SIGNATURE: &str = "prompt_too_long_compaction_recovery";
const PROMPT_TOO_LONG_COMPACTION_MESSAGE: &str = "Context limit hit. Continue from the compacted thread only. Reuse preserved carryover and avoid re-expanding the full prior context.";

pub struct Kernel {
    shell: Box<dyn Shell>,
    reasoner: Box<dyn Reasoner>,
    memory: Box<dyn Memory>,
    router: Router,
    tool_policy: ToolPolicy,
    agent_runtime: Option<Arc<dyn AgentRuntime>>,
    mcp_runtime: Option<Arc<dyn McpRuntime>>,
}

impl Kernel {
    pub fn new(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
    ) -> Result<Self> {
        Self::new_with_runtime(
            shell,
            reasoner,
            memory,
            AgentRegistrySnapshot::default(),
            ToolPolicy::allow_all(),
            None,
            None,
        )
    }

    pub fn new_with_registry(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
        registry: AgentRegistrySnapshot,
    ) -> Result<Self> {
        Self::new_with_runtime(
            shell,
            reasoner,
            memory,
            registry,
            ToolPolicy::allow_all(),
            None,
            None,
        )
    }

    pub fn new_with_registry_and_tool_policy(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
        registry: AgentRegistrySnapshot,
        tool_policy: ToolPolicy,
    ) -> Result<Self> {
        Self::new_with_runtime(shell, reasoner, memory, registry, tool_policy, None, None)
    }

    pub fn new_with_runtime(
        shell: Box<dyn Shell>,
        reasoner: Box<dyn Reasoner>,
        memory: Box<dyn Memory>,
        registry: AgentRegistrySnapshot,
        tool_policy: ToolPolicy,
        agent_runtime: Option<Arc<dyn AgentRuntime>>,
        mcp_runtime: Option<Arc<dyn McpRuntime>>,
    ) -> Result<Self> {
        Ok(Self {
            shell,
            reasoner,
            memory,
            router: Router::v1(registry),
            tool_policy,
            agent_runtime,
            mcp_runtime,
        })
    }

    pub fn route_task(&self, _task: &Task) -> RoutingDecision {
        self.active_routing_decision(&self.route_assessment(_task))
    }

    pub fn execute_task(&self, task: Task) -> Result<Outcome> {
        self.execute_task_with_config(task, ExecutionConfig::default())
    }

    pub fn execute_task_with_config(&self, task: Task, config: ExecutionConfig) -> Result<Outcome> {
        let max_steps = config.max_steps;
        self.emit_event(EventSpec::new(
            &task,
            None,
            None,
            TimelineEventType::TaskReceived,
            json!({ "task": task.description }),
        ))?;

        let routing = self.route_assessment(&task);
        let routing_decision = self.active_routing_decision(&routing);

        match routing_decision.clone() {
            RoutingDecision::HandleDirectly => {}
            RoutingDecision::RouteToExisting(agent_id) if agent_id == task.agent_id => {}
            RoutingDecision::Reactivate(agent_id) if agent_id == task.agent_id => {}
            decision => {
                let Some(runtime) = &self.agent_runtime else {
                    return Ok(Outcome::Blocked(
                        "agent routing is not available in this runtime".to_string(),
                    ));
                };
                let delegated = runtime.execute_routing_decision(
                    &RouteAgentRequest {
                        parent_task: task.clone(),
                        parent_continuation_window: task
                            .resume_context
                            .as_ref()
                            .map(|context| context.continuation_window.clone()),
                        decision,
                    },
                    config.control.as_ref(),
                )?;
                self.emit_event(EventSpec::new(
                    &task,
                    None,
                    None,
                    TimelineEventType::TaskContextAssembled,
                    json!({
                        "route": format!("{:?}", routing_decision),
                        "recommended_route": format!("{:?}", routing.recommended_decision),
                        "routing_rationale": routing.rationale,
                        "routing_candidates": routing.candidates,
                        "network_enabled": routing.network_enabled,
                        "delegated": delegated
                    }),
                ))?;
                self.emit_event(EventSpec::new(
                    &task,
                    None,
                    None,
                    TimelineEventType::TaskCompleted,
                    json!({
                        "route": format!("{:?}", routing_decision),
                        "delegated": delegated
                    }),
                ))?;
                return Ok(Outcome::Success(ActionResult::DelegatedTask(delegated)));
            }
        }

        let mut intent = Intent::from_task(&task);
        let mut state = task
            .resume_context
            .as_ref()
            .map(TaskLoopState::from_resume_context)
            .unwrap_or_else(|| TaskLoopState::new(max_steps));
        self.shell
            .restore_read_state_cache(state.read_state_cache())?;
        if state.transcript.is_empty() {
            state.record_task_message(&task.description);
        }
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::TaskContextAssembled,
            json!({
                "route": format!("{:?}", routing.effective_decision),
                "effective_route": format!("{:?}", routing_decision),
                "recommended_route": format!("{:?}", routing.recommended_decision),
                "routing_rationale": routing.rationale,
                "routing_candidates": routing.candidates,
                "network_enabled": routing.network_enabled,
                "continuation_window": self.build_active_continuation_window(
                    &task,
                    &state,
                    state.current_step().max(1),
                    max_steps,
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

        let reflex_action = select_reflex_action(
            &task,
            &self.memory.active_rules().map_err(|error| {
                KernelError::Storage(format!("failed to load active rules: {error}"))
            })?,
        );
        state.record_reflex_decision(state.current_step() + 1, reflex_action.as_ref());
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::ReflexChecked,
            json!({ "matched": reflex_action.is_some() }),
        ))?;

        let failure_count = state.circuit_breaker_failure_count();
        let tripped = state.circuit_breaker_tripped();
        state.record_circuit_breaker_state(state.current_step() + 1, failure_count, tripped);
        self.emit_event(EventSpec::new(
            &task,
            Some(&intent),
            None,
            TimelineEventType::CircuitBreakerChecked,
            json!({ "tripped": tripped, "failure_count": failure_count }),
        ))?;
        if tripped {
            let reason = "circuit breaker is tripped".to_string();
            let continuation_window = self.build_active_continuation_window(
                &task,
                &state,
                state.current_step().max(1),
                max_steps,
            );
            self.emit_event(EventSpec::new(
                &task,
                Some(&intent),
                None,
                TimelineEventType::TaskBlocked,
                json!({
                    "reason": reason,
                    "continuation_window": continuation_window
                }),
            ))?;
            return Ok(Outcome::Blocked("circuit breaker is tripped".to_string()));
        }
        let mut next_reflex_action = reflex_action;

        loop {
            if state.current_step() >= max_steps {
                let continuation_window = self.build_active_continuation_window(
                    &task,
                    &state,
                    state.current_step().max(1),
                    max_steps,
                );
                let reason = format!("step budget exhausted after {} steps", max_steps);
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    None,
                    TimelineEventType::TaskBlocked,
                    json!({
                        "reason": reason,
                        "max_steps": max_steps,
                        "continuation_window": continuation_window
                    }),
                ))?;
                return Ok(Outcome::Blocked(reason));
            }

            if let Some(max_reasoner_calls) = task_reasoner_call_budget(&task) {
                if next_reflex_action.is_none()
                    && state.model_decision_count() >= max_reasoner_calls
                {
                    let continuation_window = self.build_active_continuation_window(
                        &task,
                        &state,
                        state.current_step().max(1),
                        max_steps,
                    );
                    let reason = format!(
                        "reasoner call budget exhausted after {} calls",
                        max_reasoner_calls
                    );
                    self.emit_event(EventSpec::new(
                        &task,
                        Some(&intent),
                        None,
                        TimelineEventType::TaskBlocked,
                        json!({
                            "reason": reason,
                            "max_reasoner_calls_per_task": max_reasoner_calls,
                            "reasoner_call_budget": task_reasoner_call_budget_snapshot(
                                &task,
                                state.model_decision_count(),
                            ),
                            "continuation_window": continuation_window
                        }),
                    ))?;
                    return Ok(Outcome::Blocked(reason));
                }
            }

            if let Some(max_tokens_per_task) = task_token_budget(&task) {
                if state.reasoner_tokens_used() >= max_tokens_per_task {
                    let continuation_window = self.build_active_continuation_window(
                        &task,
                        &state,
                        state.current_step().max(1),
                        max_steps,
                    );
                    let reason = format!(
                        "reasoner token budget exhausted after {} tokens",
                        max_tokens_per_task
                    );
                    self.emit_event(EventSpec::new(
                        &task,
                        Some(&intent),
                        None,
                        TimelineEventType::TaskBlocked,
                        json!({
                            "reason": reason,
                            "max_tokens_per_task": max_tokens_per_task,
                            "reasoner_tokens_used": state.reasoner_tokens_used(),
                            "token_budget": task_token_budget_snapshot(
                                &task,
                                state.reasoner_tokens_used(),
                            ),
                            "continuation_window": continuation_window
                        }),
                    ))?;
                    return Ok(Outcome::Blocked(reason));
                }
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

            let current_step = state.current_step() + 1;
            let step = match self.select_action(
                StepSelectionContext {
                    task: &task,
                    intent: &intent,
                    state: &mut state,
                    control: config.control.as_ref(),
                    current_step,
                    max_steps,
                },
                next_reflex_action.take(),
            ) {
                Ok(step) => step,
                Err(error) => {
                    if self.try_continue_after_reasoner_recovery(
                        &task, &intent, &mut state, max_steps, &error,
                    )? {
                        continue;
                    }
                    return self
                        .finish_terminal_kernel_error(&task, &intent, &state, max_steps, error);
                }
            };
            if let Some(outcome) = self.check_cancellation(
                &task,
                Some(&intent),
                Some(&step.action),
                config.control.as_ref(),
                "before action dispatch",
            )? {
                return Ok(outcome);
            }
            let execution = match self.execute_action(
                &task,
                &mut intent,
                &state,
                &step,
                max_steps,
                config.control.as_ref(),
            ) {
                Ok(execution) => execution,
                Err(error) => {
                    return self
                        .finish_terminal_kernel_error(&task, &intent, &state, max_steps, error);
                }
            };
            let outcome = match execution {
                ActionExecution::Outcome(outcome) => outcome,
            };
            let progress = match state.record_step(&task.id, &step.action, &outcome) {
                Ok(progress) => progress,
                Err(error) => {
                    return self
                        .finish_terminal_kernel_error(&task, &intent, &state, max_steps, error);
                }
            };
            if !progress.new_content_replacements.is_empty() {
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::ContentReplacementsRecorded,
                    json!({
                        "records": progress.new_content_replacements
                    }),
                ))?;
            }
            let compaction = state.apply_live_compaction(&task.id);
            let continuation_window = self.build_active_continuation_window(
                &task,
                &state,
                state.current_step().max(1),
                max_steps,
            );

            if let Some(compaction) = compaction {
                if !compaction.new_content_replacements.is_empty() {
                    self.emit_event(EventSpec::new(
                        &task,
                        Some(&intent),
                        Some(&step.action),
                        TimelineEventType::ContentReplacementsRecorded,
                        json!({
                            "records": compaction.new_content_replacements.clone()
                        }),
                    ))?;
                }
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCompacted,
                    json!({
                        "reason": compaction.reason,
                        "score_explanations": compaction.score_explanations,
                        "compacted_results": continuation_window.reannounced_compacted_results.clone(),
                        "continuation_window": continuation_window.clone()
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
                    "continuation_window": continuation_window.clone()
                }),
            ))?;

            if progress.repeated_without_progress {
                let repeated_reason = match &step.action {
                    Action::RunCommand { .. } => "repeated a similar command family without materially changing the observed state".to_string(),
                    _ => "repeated the same step without new evidence".to_string(),
                };
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskBlocked,
                    json!({
                        "reason": repeated_reason,
                        "continuation_window": continuation_window.clone()
                    }),
                ))?;
                return Ok(Outcome::Blocked(repeated_reason));
            }

            let explicit_response = matches!(
                (&step.action, &outcome),
                (
                    Action::Respond { .. },
                    Outcome::Success(ActionResult::Response { .. })
                )
            );

            let completion_blocker = completion_blocker_reason(&task, &state.working_sources());

            if explicit_response && completion_blocker.is_none() {
                state.reset_recovery_state();
                let continuation_window = self.build_active_continuation_window(
                    &task,
                    &state,
                    state.current_step().max(1),
                    max_steps,
                );
                let final_answer_summary = match &step.action {
                    Action::Respond { message, .. } => Some(compact_answer_summary(message)),
                    _ => None,
                };
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCompleted,
                    json!({
                        "outcome": "success",
                        "final_answer_summary": final_answer_summary,
                        "continuation_window": continuation_window
                    }),
                ))?;
            }

            let tool_authored_response = if step.task_complete && completion_blocker.is_none() {
                match &outcome {
                    Outcome::Success(result) if !explicit_response => {
                        tool_authored_completion_message(result, step.framing.as_ref())
                            .map(|message| Outcome::Success(ActionResult::Response { message }))
                    }
                    _ => None,
                }
            } else {
                None
            };

            if let Some(outcome) = tool_authored_response {
                state.reset_recovery_state();
                let continuation_window = self.build_active_continuation_window(
                    &task,
                    &state,
                    state.current_step().max(1),
                    max_steps,
                );
                let final_answer_summary = match &outcome {
                    Outcome::Success(ActionResult::Response { message }) => {
                        Some(compact_answer_summary(message))
                    }
                    _ => None,
                };
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskCompleted,
                    json!({
                        "outcome": "success",
                        "completion_mode": "tool_authored",
                        "final_answer_summary": final_answer_summary,
                        "continuation_window": continuation_window
                    }),
                ))?;
                return Ok(outcome);
            }

            if let Outcome::Blocked(reason) = &outcome {
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskBlocked,
                    json!({
                        "reason": reason,
                        "continuation_window": continuation_window.clone()
                    }),
                ))?;
                return Ok(outcome);
            }

            if let Some(reason) = completion_blocker {
                let continuation_message = reason.clone();
                state.reset_recovery_state();
                state.record_guidance_update(
                    state.current_step(),
                    NextStepGuidance {
                        directive: NextStepDirective::GatherMissingFact,
                        reason,
                        based_on_action: Some(action_label(&step.action)),
                        evidence_locator: None,
                        preferred_search_family: None,
                        suggested_query: None,
                    },
                );
                state.record_transition(
                    "completion_blocker",
                    None,
                    Some(continuation_message.clone()),
                    json!({
                        "action": action_label(&step.action),
                    }),
                );
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskContinued,
                    json!({
                        "reason": "completion_blocker",
                        "message": continuation_message,
                        "action": action_label(&step.action),
                        "budget_state": transition_budget_payload(
                            &task,
                            state.model_decision_count(),
                            state.reasoner_tokens_used(),
                        ),
                        "continuation_window": self.build_active_continuation_window(
                            &task,
                            &state,
                            state.current_step().max(1),
                            max_steps,
                        )
                    }),
                ))?;
                continue;
            }

            if explicit_response {
                return Ok(outcome);
            }

            state.reset_recovery_state();
            state.record_transition(
                "next_turn",
                None,
                Some("continuing after non-terminal tool progress".to_string()),
                json!({
                    "action": action_label(&step.action),
                }),
            );
            let continuation_window = self.build_active_continuation_window(
                &task,
                &state,
                state.current_step().max(1),
                max_steps,
            );
            self.emit_event(EventSpec::new(
                &task,
                Some(&intent),
                Some(&step.action),
                TimelineEventType::TaskContinued,
                json!({
                    "reason": "next_turn",
                    "message": "continuing after non-terminal tool progress",
                    "action": action_label(&step.action),
                    "budget_state": transition_budget_payload(
                        &task,
                        state.model_decision_count(),
                        state.reasoner_tokens_used(),
                    ),
                    "continuation_window": continuation_window
                }),
            ))?;
        }
    }
}

impl Kernel {
    fn try_continue_after_reasoner_recovery(
        &self,
        task: &Task,
        intent: &Intent,
        state: &mut TaskLoopState,
        max_steps: usize,
        error: &KernelError,
    ) -> Result<bool> {
        if !is_structured_output_truncation_error(error) {
            return self.try_continue_after_prompt_too_long(task, intent, state, max_steps, error);
        }
        if state.max_output_tokens_recovery_count >= MAX_OUTPUT_TOKENS_RECOVERY_LIMIT {
            return Ok(false);
        }
        let attempt = state.max_output_tokens_recovery_count + 1;
        state.max_output_tokens_recovery_count = attempt;
        state.record_recovery_continuation(
            MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE,
            MAX_OUTPUT_TOKENS_RECOVERY_SIGNATURE,
        );
        state.record_transition(
            "max_output_tokens_recovery",
            Some(attempt as u64),
            Some(MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE.to_string()),
            serde_json::Value::Null,
        );
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            None,
            TimelineEventType::TaskRecoveryContinued,
            json!({
                "reason": "max_output_tokens_recovery",
                "attempt": attempt,
                "message": MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE,
                "budget_state": transition_budget_payload(
                    task,
                    state.model_decision_count(),
                    state.reasoner_tokens_used(),
                ),
                "continuation_window": self.build_active_continuation_window(
                    task,
                    state,
                    state.current_step().max(1),
                    max_steps,
                )
            }),
        ))?;
        Ok(true)
    }

    fn try_continue_after_prompt_too_long(
        &self,
        task: &Task,
        intent: &Intent,
        state: &mut TaskLoopState,
        max_steps: usize,
        error: &KernelError,
    ) -> Result<bool> {
        if !is_prompt_too_long_error(error) {
            return Ok(false);
        }
        if state.has_attempted_prompt_too_long_compaction {
            return Ok(false);
        }
        let Some(compaction) = state.apply_live_compaction(&task.id) else {
            return Ok(false);
        };
        state.has_attempted_prompt_too_long_compaction = true;
        state.record_recovery_continuation(
            PROMPT_TOO_LONG_COMPACTION_MESSAGE,
            PROMPT_TOO_LONG_COMPACTION_SIGNATURE,
        );
        state.record_transition(
            "prompt_too_long_compaction",
            Some(1),
            Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
            serde_json::Value::Null,
        );
        let continuation_window = self.build_active_continuation_window(
            task,
            state,
            state.current_step().max(1),
            max_steps,
        );
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            None,
            TimelineEventType::TaskRecoveryContinued,
            json!({
                "reason": "prompt_too_long_compaction",
                "attempt": 1,
                "message": PROMPT_TOO_LONG_COMPACTION_MESSAGE,
                "budget_state": transition_budget_payload(
                    task,
                    state.model_decision_count(),
                    state.reasoner_tokens_used(),
                ),
                "continuation_window": continuation_window.clone()
            }),
        ))?;
        if !compaction.new_content_replacements.is_empty() {
            self.emit_event(EventSpec::new(
                task,
                Some(intent),
                None,
                TimelineEventType::ContentReplacementsRecorded,
                json!({
                    "records": compaction.new_content_replacements.clone()
                }),
            ))?;
        }
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            None,
            TimelineEventType::TaskCompacted,
            json!({
                "reason": compaction.reason,
                "score_explanations": compaction.score_explanations,
                "compacted_results": continuation_window.reannounced_compacted_results.clone(),
                "continuation_window": continuation_window,
                "recovery": "prompt_too_long"
            }),
        ))?;
        Ok(true)
    }

    fn finish_terminal_kernel_error(
        &self,
        task: &Task,
        intent: &Intent,
        state: &TaskLoopState,
        max_steps: usize,
        error: KernelError,
    ) -> Result<Outcome> {
        let recoverable = matches!(error, KernelError::Reasoning(_) | KernelError::Execution(_));
        let reason = error.to_string();
        let continuation_window = self.build_active_continuation_window(
            task,
            state,
            state.current_step().max(1),
            max_steps,
        );
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            None,
            if recoverable {
                TimelineEventType::TaskBlocked
            } else {
                TimelineEventType::TaskFailed
            },
            json!({
                "reason": reason,
                "recoverable": recoverable,
                "continuation_window": continuation_window
            }),
        ))?;
        Ok(if recoverable {
            Outcome::Blocked(error.to_string())
        } else {
            Outcome::Failure(error.to_string())
        })
    }

    fn route_assessment(&self, task: &Task) -> RoutingAssessment {
        let latest = self.memory.agent_registry_snapshot().unwrap_or_default();
        if latest.active_agents.is_empty() && latest.archived_agents.is_empty() {
            self.router.route_task(task)
        } else {
            Router::v1(latest).route_task(task)
        }
    }

    fn active_routing_decision(&self, assessment: &RoutingAssessment) -> RoutingDecision {
        if self.agent_runtime.is_some() {
            assessment.recommended_decision.clone()
        } else {
            assessment.effective_decision.clone()
        }
    }
}

fn is_structured_output_truncation_error(error: &KernelError) -> bool {
    matches!(error, KernelError::Reasoning(message)
        if message.starts_with("Claude did not return parseable JSON.")
            || message.starts_with("invalid Claude JSON response:"))
}

fn is_prompt_too_long_error(error: &KernelError) -> bool {
    matches!(error, KernelError::Reasoning(message) if {
        let lower = message.to_lowercase();
        lower.contains("prompt too long")
            || lower.contains("prompt is too long")
            || lower.contains("request too large")
            || (lower.contains("status 413") && lower.contains("anthropic api error"))
    })
}

pub(crate) fn task_reasoner_call_budget(task: &Task) -> Option<usize> {
    task.metadata
        .get("max_reasoner_calls_per_task")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

pub(crate) fn task_reasoner_call_budget_snapshot(
    task: &Task,
    used_calls: usize,
) -> Option<serde_json::Value> {
    task_reasoner_call_budget(task).map(|budget| {
        let used = used_calls.min(budget);
        let remaining = budget.saturating_sub(used);
        let pct = ((used as u128 * 100) / budget as u128) as u64;
        json!({
            "used": used,
            "remaining": remaining,
            "budget": budget,
            "pct": pct
        })
    })
}

pub(crate) fn task_token_budget(task: &Task) -> Option<u32> {
    task.metadata
        .get("max_tokens_per_task")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
}

pub(crate) fn task_token_budget_snapshot(
    task: &Task,
    used_tokens: u32,
) -> Option<serde_json::Value> {
    task_token_budget(task).map(|budget| {
        let used = used_tokens.min(budget);
        let remaining = budget.saturating_sub(used);
        let pct = ((used as u128 * 100) / budget as u128) as u64;
        json!({
            "used": used,
            "remaining": remaining,
            "budget": budget,
            "pct": pct
        })
    })
}

pub(crate) fn transition_budget_payload(
    task: &Task,
    used_calls: usize,
    used_tokens: u32,
) -> serde_json::Value {
    json!({
        "reasoner_call_budget": task_reasoner_call_budget_snapshot(task, used_calls),
        "token_budget": task_token_budget_snapshot(task, used_tokens),
        "reasoner_tokens_used": used_tokens,
    })
}

fn compact_answer_summary(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(240).collect::<String>();
    if normalized.chars().count() > 240 {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests;
