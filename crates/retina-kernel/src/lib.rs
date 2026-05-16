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
    ActionExecution, CircuitBreaker, EventSpec, ReflexEngine, StepSelectionContext,
    tool_authored_completion_message,
};
use retina_tools::ToolPolicy;
use retina_traits::{AgentRuntime, McpRuntime, Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::sync::Arc;

pub struct Kernel {
    shell: Box<dyn Shell>,
    reasoner: Box<dyn Reasoner>,
    memory: Box<dyn Memory>,
    reflex_engine: ReflexEngine,
    circuit_breaker: CircuitBreaker,
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
        let active_rules = memory.active_rules().map_err(|error| {
            KernelError::Storage(format!("failed to load active rules: {error}"))
        })?;
        Ok(Self {
            shell,
            reasoner,
            memory,
            reflex_engine: ReflexEngine::new(active_rules),
            circuit_breaker: CircuitBreaker::default(),
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
                "task_state": self.build_task_state(
                    &task,
                    &TaskLoopState::new(max_steps),
                    1,
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

        let mut state = TaskLoopState::new(max_steps);
        let mut next_reflex_action = reflex_action;

        loop {
            if state.step_index >= max_steps {
                let task_state =
                    self.build_task_state(&task, &state, state.step_index.max(1), max_steps);
                let reason = format!("step budget exhausted after {} steps", max_steps);
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    None,
                    TimelineEventType::TaskFailed,
                    json!({
                        "reason": reason,
                        "max_steps": max_steps,
                        "task_state": task_state
                    }),
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

            let current_step = state.step_index + 1;
            let step = self.select_action(
                StepSelectionContext {
                    task: &task,
                    intent: &intent,
                    state: &mut state,
                    control: config.control.as_ref(),
                    current_step,
                    max_steps,
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
            let execution =
                self.execute_action(&task, &mut intent, &state, &step, config.control.as_ref())?;
            let outcome = match execution {
                ActionExecution::Outcome(outcome) => outcome,
            };
            let progress = state.record_step(&step.action, &outcome)?;
            let compaction = state.apply_live_compaction();
            let task_state =
                self.build_task_state(&task, &state, state.step_index.max(1), max_steps);

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
                let repeated_reason = match &step.action {
                    Action::RunCommand { .. } => "repeated a similar command family without materially changing the observed state".to_string(),
                    _ => "repeated the same step without new evidence".to_string(),
                };
                self.emit_event(EventSpec::new(
                    &task,
                    Some(&intent),
                    Some(&step.action),
                    TimelineEventType::TaskFailed,
                    json!({ "reason": repeated_reason }),
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

            if explicit_response {
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
                        "task_state": task_state
                    }),
                ))?;
            }

            let tool_authored_response = if step.task_complete {
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
                        "task_state": task_state
                    }),
                ))?;
                return Ok(outcome);
            }

            if explicit_response || matches!(outcome, Outcome::Blocked(_)) {
                return Ok(outcome);
            }
        }
    }
}

impl Kernel {
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
