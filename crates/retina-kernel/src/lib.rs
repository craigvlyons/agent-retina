mod execution;
mod loop_state;
mod result_helpers;
mod router;
mod support;
mod task_shape;

pub(crate) use crate::loop_state::{TaskLoopState, action_label};
use crate::router::Router;
pub(crate) use crate::support::{
    CircuitBreaker, EventSpec, ReflexEngine, StepSelectionContext,
};
use crate::task_shape::{
    completion_guard, infer_task_shape, suggested_step_budget,
};
use retina_traits::{Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;

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
        let initial_shape = infer_task_shape(&task.description, &TaskLoopState::new(0));
        let max_steps = suggested_step_budget(config.max_steps, &initial_shape);
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
                    &TaskLoopState::new(max_steps),
                    1,
                    max_steps,
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

        let mut state = TaskLoopState::new(max_steps);
        let mut next_reflex_action = reflex_action;

        loop {
            if state.step_index >= max_steps {
                let task_state = self.build_task_state(
                    &task,
                    &state,
                    state.step_index.max(1),
                    max_steps,
                    state.last_result_summary.clone(),
                );
                let reason = if let Some(blocker) = completion_guard(&task_state) {
                    format!(
                        "step budget exhausted after {} steps; {}",
                        max_steps, blocker
                    )
                } else {
                    format!("step budget exhausted after {} steps", max_steps)
                };
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

            let step = self.select_action(
                StepSelectionContext {
                    task: &task,
                    intent: &intent,
                    state: &state,
                    control: config.control.as_ref(),
                    current_step: state.step_index + 1,
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
            let outcome =
                self.execute_action(&task, &mut intent, &step, config.control.as_ref(), true)?;
            let progress = state.record_step(&step.action, &outcome)?;
            let compaction = state.apply_live_compaction();
            let task_state = self.build_task_state(
                &task,
                &state,
                state.step_index.max(1),
                max_steps,
                state.last_result_summary.clone(),
            );
            let completion_blocker = completion_guard(&task_state);

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
                    "completion_guard_blocked": completion_blocker,
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

            if state.record_low_value_post_ingest_exploration(&step.action, &task_state) {
                return self.reflect_or_fail(
                    &task,
                    &mut intent,
                    &step.action,
                    config.control.as_ref(),
                    "continued low-value exploration after the task already had enough context to answer, synthesize, or produce the requested result"
                        .to_string(),
                    true,
                );
            }

            if step.task_complete && completion_blocker.is_none() {
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

            if (step.task_complete && completion_blocker.is_none())
                || matches!(outcome, Outcome::Failure(_) | Outcome::Blocked(_))
            {
                return Ok(outcome);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::recover_mutex;

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
            const CONSTRAINTS: &[HardConstraint] = &[HardConstraint::DeleteOrKillRequireApproval];
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
    fn assembled_context_includes_output_task_shape() {
        let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "not done yet".to_string(),
            },
            task_complete: false,
            reasoning: Some("inspect the task shape".to_string()),
            tokens_used: TokenUsage::default(),
        }]);
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(reasoner.clone()),
            Box::new(MockMemory::default()),
        ));

        let task = Task::new(
            AgentId::new(),
            "use dominican_Med.pdf and dominican.txt to create Emily_wittenberge.txt",
        );
        let _ = must(kernel.execute_task_with_config(
            task,
            ExecutionConfig {
                max_steps: 1,
                control: None,
            },
        ));

        let seen = reasoner.seen_task_states();
        assert!(!seen.is_empty());
        assert_eq!(seen[0].shape.kind, TaskKind::Output);
        assert_eq!(seen[0].shape.required_inputs.len(), 2);
        assert_eq!(
            seen[0]
                .shape
                .requested_output
                .as_ref()
                .map(|output| output.locator_hint.as_str()),
            Some("Emily_wittenberge.txt")
        );
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
    fn discovery_only_step_cannot_finish_named_output_task() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ListDirectory {
                        id: ActionId::new(),
                        path: "Desktop".into(),
                        recursive: false,
                        max_entries: 100,
                    },
                    task_complete: true,
                    reasoning: Some("look at the desktop".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "dominican.txt".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: false,
                    reasoning: Some("read the companion text".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ExtractDocumentText {
                        id: ActionId::new(),
                        path: "dominican_Med.pdf".into(),
                        max_chars: Some(4096),
                        page_start: None,
                        page_end: None,
                    },
                    task_complete: false,
                    reasoning: Some("extract the PDF content".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::WriteFile {
                        id: ActionId::new(),
                        path: "Emily_wittenberge.txt".into(),
                        content: "filled output".to_string(),
                        overwrite: true,
                    },
                    task_complete: true,
                    reasoning: Some("write the requested output".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "take dominican_Med.pdf and dominican.txt and save as Emily_wittenberge.txt",
        )));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::FileWrite { .. })
        ));
    }

    #[test]
    fn output_tasks_receive_a_larger_default_step_budget() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ListDirectory {
                        id: ActionId::new(),
                        path: "Desktop".into(),
                        recursive: false,
                        max_entries: 100,
                    },
                    task_complete: false,
                    reasoning: Some("discover desktop files".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::FindFiles {
                        id: ActionId::new(),
                        root: "Desktop".into(),
                        pattern: "dominican_Med.pdf".to_string(),
                        max_results: 10,
                    },
                    task_complete: false,
                    reasoning: Some("locate the PDF".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "dominican.txt".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: false,
                    reasoning: Some("read the text source".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ExtractDocumentText {
                        id: ActionId::new(),
                        path: "dominican_Med.pdf".into(),
                        max_chars: Some(4096),
                        page_start: None,
                        page_end: None,
                    },
                    task_complete: false,
                    reasoning: Some("extract the PDF".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::WriteFile {
                        id: ActionId::new(),
                        path: "Emily_wittenberge.txt".into(),
                        content: "filled output".to_string(),
                        overwrite: true,
                    },
                    task_complete: true,
                    reasoning: Some("write the output".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "use dominican_Med.pdf and dominican.txt to create Emily_wittenberge.txt",
        )));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::FileWrite { .. })
        ));
    }

    #[test]
    fn output_task_fails_if_it_keeps_discovering_after_inputs_are_ingested() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "dominican.txt".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: false,
                    reasoning: Some("read the text input".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ExtractDocumentText {
                        id: ActionId::new(),
                        path: "dominican_Med.pdf".into(),
                        max_chars: Some(4096),
                        page_start: None,
                        page_end: None,
                    },
                    task_complete: false,
                    reasoning: Some("extract the pdf input".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::FindFiles {
                        id: ActionId::new(),
                        root: "Desktop".into(),
                        pattern: "other".to_string(),
                        max_results: 5,
                    },
                    task_complete: false,
                    reasoning: Some("keep exploring".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ListDirectory {
                        id: ActionId::new(),
                        path: "Desktop".into(),
                        recursive: false,
                        max_entries: 20,
                    },
                    task_complete: false,
                    reasoning: Some("still exploring".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "use dominican_Med.pdf and dominican.txt to create Emily_wittenberge.txt",
        )));
        assert!(matches!(
            outcome,
            Outcome::Failure(reason)
                if reason.contains("requested output")
                    || reason.contains("low-value exploration")
                    || reason.contains("enough context to answer, synthesize, or produce")
        ));
    }

    #[test]
    fn answer_task_cannot_finish_without_returning_a_grounded_answer() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "startup.md".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: true,
                    reasoning: Some("I have enough evidence now".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::Respond {
                        id: ActionId::new(),
                        message: "startup.md describes how to initialize the runtime and test the worker."
                            .to_string(),
                    },
                    task_complete: true,
                    reasoning: Some("return the grounded final answer".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "read startup.md and summarize it",
        )));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));
    }

    #[test]
    fn transform_task_without_named_output_cannot_finish_before_synthesis() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "startup.md".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: false,
                    reasoning: Some("read the first source".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "testing.md".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: true,
                    reasoning: Some("both sources are ingested".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::Respond {
                        id: ActionId::new(),
                        message: "startup.md covers runtime bring-up, and testing.md covers how to pressure-test the worker."
                            .to_string(),
                    },
                    task_complete: true,
                    reasoning: Some("return the transformed combined result".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "combine startup.md and testing.md into a combined summary",
        )));
        assert!(matches!(
            outcome,
            Outcome::Success(ActionResult::Response { .. })
        ));
    }

    #[test]
    fn answer_task_fails_if_it_keeps_exploring_after_inputs_are_ingested() {
        let kernel = must(Kernel::new(
            Box::new(MockShell::default()),
            Box::new(MockReasoner::sequence(vec![
                ReasonResponse {
                    action: Action::ReadFile {
                        id: ActionId::new(),
                        path: "startup.md".into(),
                        max_bytes: Some(4096),
                    },
                    task_complete: false,
                    reasoning: Some("ingest the source".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::FindFiles {
                        id: ActionId::new(),
                        root: ".".into(),
                        pattern: "testing.md".to_string(),
                        max_results: 5,
                    },
                    task_complete: false,
                    reasoning: Some("keep browsing instead of answering".to_string()),
                    tokens_used: TokenUsage::default(),
                },
                ReasonResponse {
                    action: Action::ListDirectory {
                        id: ActionId::new(),
                        path: ".".into(),
                        recursive: false,
                        max_entries: 20,
                    },
                    task_complete: false,
                    reasoning: Some("still not answering".to_string()),
                    tokens_used: TokenUsage::default(),
                },
            ])),
            Box::new(MockMemory::default()),
        ));

        let outcome = must(kernel.execute_task(Task::new(
            AgentId::new(),
            "read startup.md and tell me what it says",
        )));
        assert!(matches!(
            outcome,
            Outcome::Failure(reason)
                if reason.contains("low-value exploration")
                    || reason.contains("enough context to answer")
        ));
    }

    #[test]
    fn edit_style_task_infers_existing_file_as_requested_output() {
        let shape = infer_task_shape(
            "update startup.md using testing.md and save the revised startup.md",
            &TaskLoopState::new(6),
        );

        assert_eq!(shape.kind, TaskKind::Output);
        assert_eq!(
            shape
                .requested_output
                .as_ref()
                .map(|output| output.locator_hint.as_str()),
            Some("startup.md")
        );
        assert_eq!(shape.required_inputs.len(), 1);
        assert_eq!(shape.required_inputs[0].locator_hint, "testing.md");
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
