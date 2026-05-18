use super::*;
use retina_test_utils::{MockMemory, MockReasoner, MockShell};
use retina_tools::ToolPolicy;
use retina_traits::{AgentRuntime, McpRuntime};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tempfile::tempdir;

fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
    result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
}

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn with_test_retina_home<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = recover_mutex(ENV_LOCK.get_or_init(|| Mutex::new(())));
    let previous = std::env::var_os("RETINA_HOME");
    unsafe {
        std::env::set_var("RETINA_HOME", path);
    }
    let output = f();
    match previous {
        Some(value) => unsafe {
            std::env::set_var("RETINA_HOME", value);
        },
        None => unsafe {
            std::env::remove_var("RETINA_HOME");
        },
    }
    output
}

#[derive(Clone)]
struct GuidanceReasoner {
    seen_contexts: Arc<Mutex<Vec<AssembledContext>>>,
    seen_tools: Arc<Mutex<Vec<Vec<ToolDescriptor>>>>,
    responses: Arc<Mutex<Vec<ReasonResponse>>>,
}

impl GuidanceReasoner {
    fn new(responses: Vec<ReasonResponse>) -> Self {
        Self {
            seen_contexts: Arc::new(Mutex::new(Vec::new())),
            seen_tools: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses)),
        }
    }

    fn seen_contexts(&self) -> Vec<AssembledContext> {
        recover_mutex(&self.seen_contexts).clone()
    }

    fn seen_tools(&self) -> Vec<Vec<ToolDescriptor>> {
        recover_mutex(&self.seen_tools).clone()
    }
}

impl retina_traits::Reasoner for GuidanceReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_contexts).push(request.context.clone());
        recover_mutex(&self.seen_tools).push(request.context.tools.clone());
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
                    framing: None,
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

#[derive(Clone)]
struct FlakyReasoner {
    seen_contexts: Arc<Mutex<Vec<AssembledContext>>>,
    results: Arc<Mutex<Vec<Result<ReasonResponse>>>>,
}

impl FlakyReasoner {
    fn new(results: Vec<Result<ReasonResponse>>) -> Self {
        Self {
            seen_contexts: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(results)),
        }
    }

    fn seen_contexts(&self) -> Vec<AssembledContext> {
        recover_mutex(&self.seen_contexts).clone()
    }
}

impl retina_traits::Reasoner for FlakyReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_contexts).push(request.context.clone());
        let mut results = recover_mutex(&self.results);
        if results.len() > 1 {
            results.remove(0)
        } else {
            results.first().cloned().unwrap_or_else(|| {
                Ok(ReasonResponse {
                    action: Action::Respond {
                        id: ActionId::new(),
                        message: "fallback".to_string(),
                    },
                    task_complete: true,
                    framing: None,
                    reasoning: Some("fallback".to_string()),
                    tokens_used: TokenUsage::default(),
                })
            })
        }
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "flaky-test".to_string(),
        }
    }
}

#[derive(Clone)]
struct TransitionReasoner {
    transitions: Arc<Mutex<Vec<ReasonerTransition>>>,
}

impl TransitionReasoner {
    fn new(transitions: Vec<ReasonerTransition>) -> Self {
        Self {
            transitions: Arc::new(Mutex::new(transitions)),
        }
    }
}

impl retina_traits::Reasoner for TransitionReasoner {
    fn reason(&self, _request: &ReasonRequest) -> Result<ReasonResponse> {
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("provider transition test".to_string()),
            tokens_used: TokenUsage::default(),
        })
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "transition-test".to_string(),
        }
    }

    fn take_recent_transitions(&self) -> Vec<ReasonerTransition> {
        std::mem::take(&mut *recover_mutex(&self.transitions))
    }
}

#[derive(Clone)]
struct UsageTransitionReasoner {
    transitions: Arc<Mutex<Vec<ReasonerTransition>>>,
    response: ReasonResponse,
}

impl UsageTransitionReasoner {
    fn new(transitions: Vec<ReasonerTransition>, response: ReasonResponse) -> Self {
        Self {
            transitions: Arc::new(Mutex::new(transitions)),
            response,
        }
    }
}

impl retina_traits::Reasoner for UsageTransitionReasoner {
    fn reason(&self, _request: &ReasonRequest) -> Result<ReasonResponse> {
        Ok(self.response.clone())
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "usage-transition-test".to_string(),
        }
    }

    fn take_recent_transitions(&self) -> Vec<ReasonerTransition> {
        std::mem::take(&mut *recover_mutex(&self.transitions))
    }
}

#[derive(Clone)]
struct TransitioningFlakyReasoner {
    seen_contexts: Arc<Mutex<Vec<AssembledContext>>>,
    results: Arc<Mutex<Vec<Result<ReasonResponse>>>>,
    transitions: Arc<Mutex<Vec<Vec<ReasonerTransition>>>>,
}

impl TransitioningFlakyReasoner {
    fn new(
        results: Vec<Result<ReasonResponse>>,
        transitions: Vec<Vec<ReasonerTransition>>,
    ) -> Self {
        Self {
            seen_contexts: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(results)),
            transitions: Arc::new(Mutex::new(transitions)),
        }
    }
}

impl retina_traits::Reasoner for TransitioningFlakyReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_contexts).push(request.context.clone());
        let mut results = recover_mutex(&self.results);
        if results.len() > 1 {
            results.remove(0)
        } else {
            results.first().cloned().unwrap_or_else(|| {
                Ok(ReasonResponse {
                    action: Action::Respond {
                        id: ActionId::new(),
                        message: "fallback".to_string(),
                    },
                    task_complete: true,
                    framing: None,
                    reasoning: Some("fallback".to_string()),
                    tokens_used: TokenUsage::default(),
                })
            })
        }
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "transitioning-flaky-test".to_string(),
        }
    }

    fn take_recent_transitions(&self) -> Vec<ReasonerTransition> {
        let mut transitions = recover_mutex(&self.transitions);
        if transitions.len() > 1 {
            transitions.remove(0)
        } else {
            transitions.first().cloned().unwrap_or_default()
        }
    }
}

#[derive(Clone)]
struct MaxTokensReasoner {
    seen_max_tokens: Arc<Mutex<Vec<Option<u32>>>>,
}

impl MaxTokensReasoner {
    fn new() -> Self {
        Self {
            seen_max_tokens: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn seen_max_tokens(&self) -> Vec<Option<u32>> {
        recover_mutex(&self.seen_max_tokens).clone()
    }
}

impl retina_traits::Reasoner for MaxTokensReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_max_tokens).push(request.max_tokens);
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("max tokens test".to_string()),
            tokens_used: TokenUsage::default(),
        })
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1_000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "max-tokens-test".to_string(),
        }
    }
}

#[derive(Clone)]
struct SequenceMaxTokensReasoner {
    seen_max_tokens: Arc<Mutex<Vec<Option<u32>>>>,
    responses: Arc<Mutex<Vec<ReasonResponse>>>,
}

impl SequenceMaxTokensReasoner {
    fn new(responses: Vec<ReasonResponse>) -> Self {
        Self {
            seen_max_tokens: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses)),
        }
    }

    fn seen_max_tokens(&self) -> Vec<Option<u32>> {
        recover_mutex(&self.seen_max_tokens).clone()
    }
}

impl retina_traits::Reasoner for SequenceMaxTokensReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_max_tokens).push(request.max_tokens);
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
                    framing: None,
                    reasoning: Some("max tokens test".to_string()),
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
            model_id: "sequence-max-tokens-test".to_string(),
        }
    }
}

#[derive(Clone)]
struct MockLocalAgentRuntime {
    result: DelegatedTaskResult,
    calls: Arc<Mutex<usize>>,
    routed_calls: Arc<Mutex<usize>>,
}

#[derive(Clone, Default)]
struct MockMcpRuntime {
    snapshot: McpRegistrySnapshot,
}

impl McpRuntime for MockMcpRuntime {
    fn snapshot(&self) -> Result<McpRegistrySnapshot> {
        Ok(self.snapshot.clone())
    }

    fn list_resources(&self, _server: Option<&str>) -> Result<Vec<McpResourceSummary>> {
        Ok(Vec::new())
    }

    fn read_resource(&self, _server: &str, _uri: &str) -> Result<McpResourceReadResult> {
        Err(KernelError::Unsupported("not used in test".to_string()))
    }

    fn call_tool(
        &self,
        _server: &str,
        _tool: &str,
        _input_json: &serde_json::Value,
    ) -> Result<McpToolCallResult> {
        Err(KernelError::Unsupported("not used in test".to_string()))
    }
}

impl AgentRuntime for MockLocalAgentRuntime {
    fn spawn_local_agent(
        &self,
        _request: &SpawnAgentRequest,
        _control: Option<&ExecutionControlHandle>,
    ) -> Result<DelegatedTaskResult> {
        *recover_mutex(&self.calls) += 1;
        Ok(self.result.clone())
    }

    fn execute_routing_decision(
        &self,
        _request: &RouteAgentRequest,
        _control: Option<&ExecutionControlHandle>,
    ) -> Result<DelegatedTaskResult> {
        *recover_mutex(&self.routed_calls) += 1;
        Ok(self.result.clone())
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
fn assembled_context_includes_canonical_continuation_window() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: None,
        reasoning: Some("test".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
    ));

    let task = Task::new(AgentId::new(), "read startup.md");
    let _ = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].continuation_window.objective, "read startup.md");
    assert_eq!(seen[0].current_step, 1);
}

#[test]
fn initial_reasoner_context_filters_control_only_transcript_units() {
    let memory = MockMemory::default();
    must(memory.store_rule(&ReflexiveRule {
        id: Some(RuleId::new()),
        name: "startup reflex".to_string(),
        condition: RuleCondition::TaskContains("startup".to_string()),
        action: RuleAction::UseAction(Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }),
        confidence: 1.0,
        active: true,
        last_fired: None,
    }));
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: None,
        reasoning: Some("test".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory),
    ));

    let _ = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "read startup.md"),
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 1);
    let transcript = seen[0].continuation_window.transcript.entries();
    assert!(
        transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::TaskMessage))
    );
    assert!(
        transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::ToolInvocation))
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::ReflexDecision))
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::CircuitBreakerState))
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::OperatorGuidance))
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::GuidanceUpdate))
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::ModelDecision))
    );
}

#[test]
fn resumed_task_starts_with_prior_continuation_window() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: None,
        reasoning: Some("continue from saved state".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
    ));

    let source_task = Task::new(AgentId::new(), "finish the combined report");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "finish the combined report".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "finish the combined report".to_string(),
                current_step: 6,
                max_steps: 50,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 6,
                    kind: TranscriptUnitKind::ToolResult,
                    summary: "looked up company background".to_string(),
                    result_ref_id: Some("result-6-1".to_string()),
                    primary_locator: Some("https://example.com/company".to_string()),
                    evidence_refs: vec!["https://example.com/company".to_string()],
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                stored_results: StoredResultLedger::from_entries(vec![StoredResultReference {
                    result_id: "result-6-1".to_string(),
                    source_transcript_ordinal: 1,
                    step: 6,
                    result_type: "mcp_tool_call".to_string(),
                    primary_locator: Some("https://example.com/company".to_string()),
                    preview_excerpt: "company background".to_string(),
                    persisted_path: "/tmp/result-6-1.json".to_string(),
                }]),
                content_replacements: ContentReplacementState {
                    records: vec![ContentReplacementRecord {
                        replacement_id: "result-6-1".to_string(),
                        source_kind: "stored_result".to_string(),
                        result_type: "mcp_tool_call".to_string(),
                        locator: Some("https://example.com/company".to_string()),
                        persisted_path: Some("/tmp/result-6-1.json".to_string()),
                        replacement_text: "[stored-result result-6-1] frozen exact replacement"
                            .to_string(),
                    }],
                },
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "reasoning transport failed".to_string(),
        },
        None,
    );

    let _ = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 50,
            control: None,
        },
    ));

    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].continuation_window.current_step, 7);
    assert!(
        seen[0]
            .continuation_window
            .transcript
            .entries()
            .iter()
            .any(|unit| unit.summary.contains("looked up company background"))
    );
    assert!(
        seen[0]
            .continuation_window
            .stored_results
            .entries()
            .iter()
            .any(|item| item.result_id == "result-6-1")
    );
    assert_eq!(
        seen[0].continuation_window.content_replacements.records[0].replacement_text,
        "[stored-result result-6-1] frozen exact replacement"
    );
}

#[test]
fn structured_output_truncation_continues_same_turn_with_recovery_message() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "Claude did not return parseable JSON. Raw response: {\"type\":\"write_file\""
                .to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("recovered after truncation".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory.clone()),
    ));

    let task = Task::new(AgentId::new(), "write the combined report");
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1]
            .continuation_window
            .transcript
            .entries()
            .iter()
            .any(
                |unit| matches!(unit.kind, TranscriptUnitKind::RecoveryContinuation)
                    && unit.summary.contains("Output token limit hit")
            )
    );
    let events = must(memory.recent_states(20));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        })
        .expect("recovery transition event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    let completed = events
        .iter()
        .find(|event| {
            event.task_id == task.id && matches!(event.event_type, TimelineEventType::TaskCompleted)
        })
        .expect("completed event should exist");
    assert_eq!(
        completed
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
}

#[test]
fn structured_output_truncation_blocks_after_recovery_limit() {
    let reasoner = FlakyReasoner::new(vec![Err(KernelError::Reasoning(
        "invalid Claude JSON response: EOF while parsing a string".to_string(),
    ))]);
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "save one large combined report"),
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    assert_eq!(reasoner.seen_contexts().len(), 4);
}

#[test]
fn resumed_task_does_not_retry_truncation_after_recovery_limit() {
    let reasoner = FlakyReasoner::new(vec![Err(KernelError::Reasoning(
        "invalid Claude JSON response: EOF while parsing a string".to_string(),
    ))]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(AgentId::new(), "resume after repeated truncation");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume after repeated truncation".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume after repeated truncation".to_string(),
                current_step: 2,
                max_steps: 10,
                max_output_tokens_recovery_count: 3,
                last_transition: Some(ContinuationTransition {
                    reason: "max_output_tokens_recovery".to_string(),
                    attempt: Some(3),
                    message: Some(MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "resume after repeated truncation".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after third truncation recovery".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 10,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    assert_eq!(reasoner.seen_contexts().len(), 1);
    let events = must(memory.recent_states(20));
    assert!(
        !events.iter().any(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        }),
        "resumed task should not emit another recovery continuation after the limit"
    );
}

#[test]
fn truncation_recovery_resets_after_next_turn_progress() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "invalid Claude JSON response: EOF while parsing a string".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::RunCommand {
                id: ActionId::new(),
                command: "pwd".to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            },
            task_complete: false,
            framing: None,
            reasoning: Some("inspect first".to_string()),
            tokens_used: TokenUsage::default(),
        }),
        Err(KernelError::Reasoning(
            "invalid Claude JSON response: EOF while parsing a string".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let task = Task::new(AgentId::new(), "recover, make progress, then recover again");
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 4,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(60));
    let recoveries: Vec<_> = events
        .iter()
        .filter(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("max_output_tokens_recovery")
        })
        .collect();
    assert_eq!(recoveries.len(), 2);
    assert_eq!(
        recoveries[0]
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        recoveries[1]
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[test]
fn prompt_too_long_reactively_compacts_and_continues() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "Anthropic API error (status 413): request too large".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("recovered after compaction".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(AgentId::new(), "finish the combined report");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "finish the combined report".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "finish the combined report".to_string(),
                current_step: 4,
                max_steps: 50,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "finish the combined report".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 2,
                        kind: TranscriptUnitKind::ToolInvocation,
                        summary: "read_file".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 3,
                        step: 2,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source A".to_string(),
                        result_ref_id: Some("result-2".to_string()),
                        primary_locator: Some("/tmp/source-a.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-a.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the combined report".to_string(),
                            last_used_step: 2,
                            evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source a".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 4,
                        step: 3,
                        kind: TranscriptUnitKind::ToolInvocation,
                        summary: "read_file".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 5,
                        step: 3,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source B".to_string(),
                        result_ref_id: Some("result-3".to_string()),
                        primary_locator: Some("/tmp/source-b.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-b.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the combined report".to_string(),
                            last_used_step: 3,
                            evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source b".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 6,
                        step: 4,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "wrote draft report".to_string(),
                        result_ref_id: Some("result-4".to_string()),
                        primary_locator: Some("/tmp/report.md".to_string()),
                        evidence_refs: vec!["/tmp/report.md".to_string()],
                        working_sources: Vec::new(),
                        artifact_references: vec![ArtifactReference {
                            locator: "/tmp/report.md".to_string(),
                            kind: "file".to_string(),
                            status: "created".to_string(),
                        }],
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                stored_results: StoredResultLedger::default(),
                content_replacements: ContentReplacementState::default(),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "prompt too long".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 50,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1]
            .continuation_window
            .transcript
            .entries()
            .iter()
            .any(
                |unit| matches!(unit.kind, TranscriptUnitKind::RecoveryContinuation)
                    && unit.summary.contains("Context limit hit")
            )
    );
    assert_eq!(seen[1].continuation_window.compaction_boundaries.len(), 1);
    let events = must(memory.recent_states(40));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        })
        .expect("recovery transition event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    let completed = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskCompleted)
        })
        .expect("completed event should exist");
    assert_eq!(
        completed
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]
fn prompt_too_long_recovery_resets_after_next_turn_progress() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "Anthropic API error (status 413): request too large".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::RunCommand {
                id: ActionId::new(),
                command: "pwd".to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            },
            task_complete: false,
            framing: None,
            reasoning: Some("inspect the compacted state".to_string()),
            tokens_used: TokenUsage::default(),
        }),
        Err(KernelError::Reasoning(
            "Anthropic API error (status 413): request too large".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(
        AgentId::new(),
        "recover from prompt-too-long twice with progress in between",
    );
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "recover from prompt-too-long twice with progress in between".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "recover from prompt-too-long twice with progress in between"
                    .to_string(),
                current_step: 4,
                max_steps: 50,
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "recover from prompt-too-long twice with progress in between"
                            .to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 2,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source A".to_string(),
                        result_ref_id: Some("result-2".to_string()),
                        primary_locator: Some("/tmp/source-a.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-a.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the report".to_string(),
                            last_used_step: 2,
                            evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source a".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 3,
                        step: 3,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source B".to_string(),
                        result_ref_id: Some("result-3".to_string()),
                        primary_locator: Some("/tmp/source-b.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-b.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the report".to_string(),
                            last_used_step: 3,
                            evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source b".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 4,
                        step: 4,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "draft artifact exists".to_string(),
                        result_ref_id: Some("result-4".to_string()),
                        primary_locator: Some("/tmp/report.md".to_string()),
                        evidence_refs: vec!["/tmp/report.md".to_string()],
                        working_sources: Vec::new(),
                        artifact_references: vec![ArtifactReference {
                            locator: "/tmp/report.md".to_string(),
                            kind: "file".to_string(),
                            status: "created".to_string(),
                        }],
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 50,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(80));
    let recoveries: Vec<_> = events
        .iter()
        .filter(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("prompt_too_long_compaction")
        })
        .collect();
    assert_eq!(recoveries.len(), 2);
    assert_eq!(
        recoveries[0]
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        recoveries[1]
            .payload_json
            .get("attempt")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[test]
fn resumed_task_does_not_retry_prompt_too_long_compaction_after_attempt() {
    let reasoner = FlakyReasoner::new(vec![Err(KernelError::Reasoning(
        "Anthropic API error (status 413): request too large".to_string(),
    ))]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(AgentId::new(), "resume after prompt-too-long compaction");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume after prompt-too-long compaction".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume after prompt-too-long compaction".to_string(),
                current_step: 3,
                max_steps: 10,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "resume after prompt-too-long compaction".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after reactive compaction retry".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 10,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    assert_eq!(reasoner.seen_contexts().len(), 1);
    let events = must(memory.recent_states(20));
    assert!(
        !events.iter().any(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        }),
        "resumed task should not emit another prompt-too-long recovery continuation"
    );
    assert!(
        !events.iter().any(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskCompacted)
        }),
        "resumed task should not compact again after the reactive compaction guard is set"
    );
}

#[test]
fn truncation_recovery_preserves_prompt_too_long_guard() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "invalid Claude JSON response: EOF while parsing a string".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish after truncation recovery".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(
        AgentId::new(),
        "resume after prompt-too-long then truncation",
    );
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume after prompt-too-long then truncation".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume after prompt-too-long then truncation".to_string(),
                current_step: 3,
                max_steps: 10,
                max_output_tokens_recovery_count: 1,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "resume after prompt-too-long then truncation".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after prompt compaction".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 10,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(20));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("max_output_tokens_recovery")
        })
        .expect("truncation recovery event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn reasoner_side_transitions_are_emitted_as_recovery_events() {
    let reasoner = TransitionReasoner::new(vec![ReasonerTransition {
        reason: "max_output_tokens_escalate".to_string(),
        message: Some("Retrying the same request with a larger output token budget.".to_string()),
        attempt: Some(1),
        metadata: json!({ "max_tokens": 64_000 }),
    }]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "write the combined report");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "2048".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(20));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        })
        .expect("provider transition event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
    assert_eq!(
        recovery
            .payload_json
            .get("metadata")
            .and_then(|value| value.get("max_tokens"))
            .and_then(|value| value.as_u64()),
        Some(64_000)
    );
    assert_eq!(
        recovery
            .payload_json
            .get("budget_state")
            .and_then(|value| value.get("token_budget"))
            .and_then(|value| value.get("budget"))
            .and_then(|value| value.as_u64()),
        Some(2048)
    );
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
}

#[test]
fn prompt_too_long_recovery_preserves_truncation_recovery_count() {
    let reasoner = FlakyReasoner::new(vec![
        Err(KernelError::Reasoning(
            "Anthropic API error (status 413): request too large".to_string(),
        )),
        Ok(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish after compaction recovery".to_string()),
            tokens_used: TokenUsage::default(),
        }),
    ]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(
        AgentId::new(),
        "resume after truncation then prompt-too-long",
    );
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume after truncation then prompt-too-long".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume after truncation then prompt-too-long".to_string(),
                current_step: 4,
                max_steps: 50,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 2,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: Some(ContinuationTransition {
                    reason: "max_output_tokens_recovery".to_string(),
                    attempt: Some(2),
                    message: Some(MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "resume after truncation then prompt-too-long".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 2,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source A".to_string(),
                        result_ref_id: Some("result-2".to_string()),
                        primary_locator: Some("/tmp/source-a.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-a.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the report".to_string(),
                            last_used_step: 2,
                            evidence_refs: vec!["/tmp/source-a.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source a".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 3,
                        step: 3,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "looked up source B".to_string(),
                        result_ref_id: Some("result-3".to_string()),
                        primary_locator: Some("/tmp/source-b.txt".to_string()),
                        evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                        working_sources: vec![WorkingSource {
                            locator: "/tmp/source-b.txt".to_string(),
                            kind: "file".to_string(),
                            role: "authoritative".to_string(),
                            status: "read".to_string(),
                            why_it_matters: "needed for the report".to_string(),
                            last_used_step: 3,
                            evidence_refs: vec!["/tmp/source-b.txt".to_string()],
                            page_reference: None,
                            extraction_method: None,
                            structured_summary: None,
                            preview_excerpt: Some("source b".to_string()),
                        }],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 4,
                        step: 4,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "draft artifact exists".to_string(),
                        result_ref_id: Some("result-4".to_string()),
                        primary_locator: Some("/tmp/report.md".to_string()),
                        evidence_refs: vec!["/tmp/report.md".to_string()],
                        working_sources: Vec::new(),
                        artifact_references: vec![ArtifactReference {
                            locator: "/tmp/report.md".to_string(),
                            kind: "file".to_string(),
                            status: "created".to_string(),
                        }],
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after truncation recovery".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 50,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(40));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("prompt_too_long_compaction")
        })
        .expect("prompt-too-long recovery event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn provider_escalation_budget_state_reflects_pre_response_usage() {
    let reasoner = UsageTransitionReasoner::new(
        vec![ReasonerTransition {
            reason: "max_output_tokens_escalate".to_string(),
            message: Some(
                "Retrying the same request with a larger output token budget.".to_string(),
            ),
            attempt: Some(1),
            metadata: json!({ "max_tokens": 64_000 }),
        }],
        ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 300,
                output_tokens: 200,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
    );
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "write the combined report");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "2048".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(20));
    let recovery = events
        .iter()
        .find(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("max_output_tokens_escalate")
        })
        .expect("provider transition event should exist");
    assert_eq!(
        recovery
            .payload_json
            .get("budget_state")
            .and_then(|value| value.get("token_budget"))
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    let reasoner_called = events
        .iter()
        .find(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::ReasonerCalled)
        })
        .expect("reasoner called event should exist");
    assert_eq!(
        reasoner_called
            .payload_json
            .get("cumulative_tokens")
            .and_then(|value| value.as_u64()),
        Some(500)
    );
}

#[test]
fn provider_escalation_preserves_other_recovery_family_state() {
    let reasoner = UsageTransitionReasoner::new(
        vec![ReasonerTransition {
            reason: "max_output_tokens_escalate".to_string(),
            message: Some(
                "Retrying the same request with a larger output token budget.".to_string(),
            ),
            attempt: Some(1),
            metadata: json!({ "max_tokens": 64_000 }),
        }],
        ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage::default(),
        },
    );
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(AgentId::new(), "resume with both recovery family state");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume with both recovery family state".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume with both recovery family state".to_string(),
                current_step: 2,
                max_steps: 5,
                max_output_tokens_recovery_count: 2,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "resume with both recovery family state".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after mixed recovery history".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 5,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(20));
    let escalation = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("max_output_tokens_escalate")
        })
        .expect("provider escalation event should exist");
    assert_eq!(
        escalation
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(2)
    );
    assert_eq!(
        escalation
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn token_budget_block_keeps_latest_continuation_transition() {
    let reasoner = UsageTransitionReasoner::new(
        vec![ReasonerTransition {
            reason: "max_output_tokens_escalate".to_string(),
            message: Some(
                "Retrying the same request with a larger output token budget.".to_string(),
            ),
            attempt: Some(1),
            metadata: json!({ "max_tokens": 64_000 }),
        }],
        ReasonResponse {
            action: Action::RunCommand {
                id: ActionId::new(),
                command: "pwd".to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            },
            task_complete: false,
            framing: None,
            reasoning: Some("inspect first".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 300,
                output_tokens: 200,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
    );
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "write the combined report");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "500".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(20));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == task.id && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("token budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("next_turn")
    );
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("reasoner_tokens_used"))
            .and_then(|value| value.as_u64()),
        Some(500)
    );
}

#[test]
fn provider_escalation_transition_survives_error_and_precedes_recovery() {
    let reasoner = TransitioningFlakyReasoner::new(
        vec![
            Err(KernelError::Reasoning(
                "invalid Claude JSON response: EOF while parsing a string".to_string(),
            )),
            Ok(ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish after recovery".to_string()),
                tokens_used: TokenUsage::default(),
            }),
        ],
        vec![
            vec![ReasonerTransition {
                reason: "max_output_tokens_escalate".to_string(),
                message: Some(
                    "Retrying the same request with a larger output token budget.".to_string(),
                ),
                attempt: Some(1),
                metadata: json!({ "max_tokens": 64_000 }),
            }],
            Vec::new(),
        ],
    );
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let task = Task::new(AgentId::new(), "write the combined report");
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(30));
    let recoveries: Vec<_> = events
        .iter()
        .filter(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
        })
        .collect();
    assert_eq!(recoveries.len(), 2);
    let escalation = recoveries
        .iter()
        .find(|event| {
            event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                == Some("max_output_tokens_escalate")
        })
        .expect("provider escalation transition event should exist");
    let recovery = recoveries
        .iter()
        .find(|event| {
            event
                .payload_json
                .get("reason")
                .and_then(|value| value.as_str())
                == Some("max_output_tokens_recovery")
        })
        .expect("follow-up truncation recovery event should exist");
    assert_eq!(
        escalation
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
    assert_eq!(
        recovery
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_recovery")
    );
    assert_eq!(
        escalation
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
    assert_eq!(
        escalation
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        recovery
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[test]
fn provider_escalation_remains_last_transition_when_recovery_is_exhausted() {
    let reasoner = TransitioningFlakyReasoner::new(
        vec![Err(KernelError::Reasoning(
            "invalid Claude JSON response: EOF while parsing a string".to_string(),
        ))],
        vec![vec![ReasonerTransition {
            reason: "max_output_tokens_escalate".to_string(),
            message: Some(
                "Retrying the same request with a larger output token budget.".to_string(),
            ),
            attempt: Some(1),
            metadata: json!({ "max_tokens": 64_000 }),
        }]],
    );
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let source_task = Task::new(AgentId::new(), "resume with exhausted truncation recovery");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id,
            source_session_id: source_task.session_id,
            source_agent_id: source_task.agent_id,
            objective: "resume with exhausted truncation recovery".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "resume with exhausted truncation recovery".to_string(),
                current_step: 2,
                max_steps: 5,
                max_output_tokens_recovery_count: 3,
                last_transition: Some(ContinuationTransition {
                    reason: "max_output_tokens_recovery".to_string(),
                    attempt: Some(3),
                    message: Some(MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "resume with exhausted truncation recovery".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after recovery limit".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 5,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(20));
    let escalation = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskRecoveryContinued)
                && event
                    .payload_json
                    .get("reason")
                    .and_then(|value| value.as_str())
                    == Some("max_output_tokens_escalate")
        })
        .expect("provider escalation transition should exist");
    assert_eq!(
        escalation
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("blocked event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_escalate")
    );
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(3)
    );
}

#[test]
fn completion_blocker_emits_task_continued_event() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: Some(ReasonerTaskFraming {
            intent_kind: Some(TaskKind::Output),
            deliverable: Some("master_summary.txt".to_string()),
            completion_basis: Some("written and verified".to_string()),
        }),
        reasoning: Some("finish".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: TaskId::new(),
            source_session_id: SessionId::new(),
            source_agent_id: AgentId::new(),
            objective: "review all files in the folder and save one master summary".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "review all files in the folder and save one master summary".to_string(),
                current_step: 1,
                max_steps: 2,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "review all files in the folder and save one master summary"
                            .to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 1,
                        kind: TranscriptUnitKind::CarryoverMessage,
                        summary: "candidate inputs discovered".to_string(),
                        result_ref_id: None,
                        primary_locator: Some("/tmp/a.pdf".to_string()),
                        evidence_refs: vec!["/tmp/a.pdf".to_string(), "/tmp/b.pdf".to_string()],
                        working_sources: vec![
                            WorkingSource {
                                locator: "/tmp/a.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "candidate".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "batch input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/a.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("a".to_string()),
                            },
                            WorkingSource {
                                locator: "/tmp/b.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "candidate".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "batch input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/b.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("b".to_string()),
                            },
                            WorkingSource {
                                locator: "/tmp/a.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "authoritative".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "covered input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/a.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("a".to_string()),
                            },
                        ],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue".to_string(),
        },
        None,
    );

    let outcome = must(kernel.execute_task_with_config(
        resumed.clone(),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(40));
    let continued = events
        .iter()
        .find(|event| {
            event.task_id == resumed.id
                && matches!(event.event_type, TimelineEventType::TaskContinued)
        })
        .expect("completion blocker continuation event should exist");
    assert_eq!(
        continued
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("completion_blocker")
    );
}

#[test]
fn completion_blocker_resets_recovery_state() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: Some(ReasonerTaskFraming {
            intent_kind: Some(TaskKind::Output),
            deliverable: Some("master_summary.txt".to_string()),
            completion_basis: Some("written and verified".to_string()),
        }),
        reasoning: Some("finish".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner),
        Box::new(memory.clone()),
    ));

    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: TaskId::new(),
            source_session_id: SessionId::new(),
            source_agent_id: AgentId::new(),
            objective: "review all files in the folder and save one master summary".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "review all files in the folder and save one master summary".to_string(),
                current_step: 1,
                max_steps: 2,
                max_output_tokens_recovery_count: 2,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "review all files in the folder and save one master summary"
                            .to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 1,
                        kind: TranscriptUnitKind::CarryoverMessage,
                        summary: "candidate inputs discovered".to_string(),
                        result_ref_id: None,
                        primary_locator: Some("/tmp/a.pdf".to_string()),
                        evidence_refs: vec!["/tmp/a.pdf".to_string(), "/tmp/b.pdf".to_string()],
                        working_sources: vec![
                            WorkingSource {
                                locator: "/tmp/a.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "candidate".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "batch input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/a.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("a".to_string()),
                            },
                            WorkingSource {
                                locator: "/tmp/b.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "candidate".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "batch input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/b.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("b".to_string()),
                            },
                            WorkingSource {
                                locator: "/tmp/a.pdf".to_string(),
                                kind: "file".to_string(),
                                role: "authoritative".to_string(),
                                status: "read".to_string(),
                                why_it_matters: "covered input".to_string(),
                                last_used_step: 1,
                                evidence_refs: vec!["/tmp/a.pdf".to_string()],
                                page_reference: None,
                                extraction_method: None,
                                structured_summary: None,
                                preview_excerpt: Some("a".to_string()),
                            },
                        ],
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue".to_string(),
        },
        None,
    );

    let outcome = must(kernel.execute_task_with_config(
        resumed.clone(),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(40));
    let continued = events
        .iter()
        .find(|event| {
            event.task_id == resumed.id
                && matches!(event.event_type, TimelineEventType::TaskContinued)
        })
        .expect("completion blocker continuation event should exist");
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("completion_blocker")
    );
}

#[test]
fn non_terminal_progress_emits_next_turn_continuation_event() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "pwd".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "inspect then answer");
    task.metadata
        .insert("max_reasoner_calls_per_task".to_string(), "4".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(50));
    let continued = events
        .iter()
        .find(|event| {
            event.task_id == task.id && matches!(event.event_type, TimelineEventType::TaskContinued)
        })
        .expect("next turn continuation event should exist");
    assert_eq!(
        continued
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("next_turn")
    );
    assert_eq!(
        continued
            .payload_json
            .get("budget_state")
            .and_then(|value| value.get("reasoner_call_budget"))
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("next_turn")
    );
}

#[test]
fn next_turn_resets_carried_recovery_state() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "pwd".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: TaskId::new(),
            source_session_id: SessionId::new(),
            source_agent_id: AgentId::new(),
            objective: "inspect then answer".to_string(),
            continuation_window: ActiveContinuationWindow {
                objective: "inspect then answer".to_string(),
                current_step: 1,
                max_steps: 3,
                max_output_tokens_recovery_count: 2,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 1,
                    kind: TranscriptUnitKind::TaskMessage,
                    summary: "inspect then answer".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: Vec::new(),
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after earlier recovery".to_string(),
        },
        None,
    );

    let outcome = must(kernel.execute_task_with_config(
        resumed.clone(),
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(50));
    let continued = events
        .iter()
        .find(|event| {
            event.task_id == resumed.id
                && matches!(event.event_type, TimelineEventType::TaskContinued)
        })
        .expect("next turn continuation event should exist");
    assert_eq!(
        continued
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("next_turn")
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        continued
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("next_turn")
    );
}

#[test]
fn task_metadata_max_tokens_budget_overrides_reasoner_heuristic() {
    let reasoner = MaxTokensReasoner::new();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
    ));

    let mut task = Task::new(AgentId::new(), "write a combined report");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "2048".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    assert_eq!(reasoner.seen_max_tokens(), vec![Some(2048)]);
}

#[test]
fn later_reasoner_calls_use_remaining_task_token_budget() {
    let reasoner = SequenceMaxTokensReasoner::new(vec![
        ReasonResponse {
            action: Action::RunCommand {
                id: ActionId::new(),
                command: "pwd".to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            },
            task_complete: false,
            framing: None,
            reasoning: Some("inspect first".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 300,
                output_tokens: 300,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
        ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage::default(),
        },
    ]);
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
    ));

    let mut task = Task::new(AgentId::new(), "inspect then answer");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "1000".to_string());
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    assert_eq!(reasoner.seen_max_tokens(), vec![Some(1000), Some(400)]);
}

#[test]
fn reasoner_call_budget_blocks_before_second_model_decision() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "pwd".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "inspect then answer");
    task.metadata
        .insert("max_reasoner_calls_per_task".to_string(), "1".to_string());
    let task_id = task.id.clone();
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(50));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == task_id && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("reasoner call budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("max_reasoner_calls_per_task")
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("reasoner call budget exhausted after 1 calls")
    );
}

#[test]
fn cumulative_reasoner_token_budget_blocks_before_next_turn() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "pwd".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage {
                    input_tokens: 600,
                    output_tokens: 500,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "inspect then answer");
    task.metadata
        .insert("max_tokens_per_task".to_string(), "1000".to_string());
    let task_id = task.id.clone();
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(50));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == task_id && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("token budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("max_tokens_per_task")
            .and_then(|value| value.as_u64()),
        Some(1000)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("reasoner_tokens_used")
            .and_then(|value| value.as_u64()),
        Some(1100)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("budget"))
            .and_then(|value| value.as_u64()),
        Some(1000)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(1000)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("remaining"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("reason")
            .and_then(|value| value.as_str()),
        Some("reasoner token budget exhausted after 1000 tokens")
    );
}

#[test]
fn resumed_task_carries_forward_reasoner_token_budget_usage() {
    let memory = MockMemory::default();
    let source_task = Task::new(AgentId::new(), "resume and finish");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id.clone(),
            source_session_id: source_task.session_id.clone(),
            source_agent_id: source_task.agent_id.clone(),
            objective: source_task.description.clone(),
            continuation_window: ActiveContinuationWindow {
                objective: source_task.description.clone(),
                current_step: 1,
                max_steps: 3,
                reasoner_tokens_used: 900,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after interruption".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();

    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "pwd".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage {
                    input_tokens: 150,
                    output_tokens: 50,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let mut resumed = resumed;
    resumed
        .metadata
        .insert("max_tokens_per_task".to_string(), "1000".to_string());
    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(50));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("resumed token budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("reasoner_tokens_used")
            .and_then(|value| value.as_u64()),
        Some(1100)
    );
    assert_eq!(
        blocked
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(1000)
    );
}

#[test]
fn reasoner_called_event_carries_structured_budget_state() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "done".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("finish".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 300,
                output_tokens: 200,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        }])),
        Box::new(memory.clone()),
    ));

    let mut task = Task::new(AgentId::new(), "answer once");
    task.metadata
        .insert("max_reasoner_calls_per_task".to_string(), "4".to_string());
    task.metadata
        .insert("max_tokens_per_task".to_string(), "1000".to_string());
    let task_id = task.id.clone();
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let events = must(memory.recent_states(50));
    let reasoner_called = events
        .iter()
        .find(|event| {
            event.task_id == task_id
                && matches!(event.event_type, TimelineEventType::ReasonerCalled)
        })
        .expect("reasoner-called event should exist");
    assert_eq!(
        reasoner_called
            .payload_json
            .get("reasoner_call_budget")
            .and_then(|value| value.get("budget"))
            .and_then(|value| value.as_u64()),
        Some(4)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("reasoner_call_budget")
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("reasoner_call_budget")
            .and_then(|value| value.get("remaining"))
            .and_then(|value| value.as_u64()),
        Some(3)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("budget"))
            .and_then(|value| value.as_u64()),
        Some(1000)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("used"))
            .and_then(|value| value.as_u64()),
        Some(500)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("remaining"))
            .and_then(|value| value.as_u64()),
        Some(500)
    );
    assert_eq!(
        reasoner_called
            .payload_json
            .get("token_budget")
            .and_then(|value| value.get("pct"))
            .and_then(|value| value.as_u64()),
        Some(50)
    );
}

#[test]
fn assembled_context_includes_output_task_shape() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "not done yet".to_string(),
        },
        task_complete: false,
        framing: None,
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

    let seen = reasoner.seen_contexts();
    assert!(!seen.is_empty());
    assert_eq!(
        seen[0].continuation_window.objective,
        "use dominican_Med.pdf and dominican.txt to create Emily_wittenberge.txt"
    );
}

#[test]
fn assembled_context_exposes_callable_tools_not_memory_records() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: None,
        reasoning: Some("tool visibility check".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let memory = MockMemory::default();
    must(memory.register_tool(&ToolRecord {
        id: None,
        name: "custom_remote_tool".to_string(),
        description: "not callable through the current runtime".to_string(),
        source_lang: SourceLanguage::Other("typescript".to_string()),
        test_status: "passing".to_string(),
        metadata: serde_json::json!({ "callable": false }),
    }));
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(memory),
    ));

    let task = Task::new(AgentId::new(), "read startup.md");
    let _ = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    let tools = reasoner.seen_tools().into_iter().next().unwrap_or_default();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"append_file"));
    assert!(names.contains(&"record_note"));
    assert!(!names.contains(&"custom_remote_tool"));
}

#[test]
fn invalid_mcp_fileish_read_is_recorded_as_failed_step() {
    let reasoner = GuidanceReasoner::new(vec![
        ReasonResponse {
            action: Action::ReadFile {
                id: ActionId::new(),
                path: "brave/brave_web_search".into(),
                start_line: None,
                limit_lines: None,
                max_bytes: None,
            },
            task_complete: false,
            framing: None,
            reasoning: Some("try opening the MCP result".to_string()),
            tokens_used: TokenUsage::default(),
        },
        ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "answered from the MCP result".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("recover after bad MCP file step".to_string()),
            tokens_used: TokenUsage::default(),
        },
    ]);
    let kernel = must(Kernel::new_with_runtime(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
        AgentRegistrySnapshot::default(),
        ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: false,
            allow_file_reads: true,
            allow_file_writes: false,
            allow_file_search: false,
            allow_mcp: true,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        }),
        None,
        Some(Arc::new(MockMcpRuntime {
            snapshot: McpRegistrySnapshot {
                servers: vec![McpServerSnapshot {
                    name: "brave".to_string(),
                    tools: vec![McpToolSummary {
                        server: "brave".to_string(),
                        name: "brave_web_search".to_string(),
                        description: Some("Search the web".to_string()),
                        input_schema: serde_json::json!({
                            "type": "object",
                            "properties": {
                                "query": { "type": "string" }
                            },
                            "required": ["query"]
                        }),
                        read_only: true,
                        destructive: false,
                        open_world: true,
                    }],
                    resources: Vec::new(),
                    error: None,
                }],
            },
        })),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(
            AgentId::new(),
            "search the web for what is happening this weekend",
        ),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 2);
    let last = seen[1]
        .continuation_window
        .transcript
        .entries()
        .iter()
        .rev()
        .find(|entry| matches!(entry.kind, TranscriptUnitKind::TerminalFailure))
        .unwrap_or_else(|| panic!("expected terminal failure entry"));
    assert!(last.summary.contains("mcp-tool://brave/brave_web_search"));
}

#[test]
fn authority_backed_tool_policy_filters_prompt_tool_pool() {
    let reasoner = GuidanceReasoner::new(vec![ReasonResponse {
        action: Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        },
        task_complete: true,
        framing: None,
        reasoning: Some("authority policy check".to_string()),
        tokens_used: TokenUsage::default(),
    }]);
    let kernel = must(Kernel::new_with_registry_and_tool_policy(
        Box::new(MockShell::default()),
        Box::new(reasoner.clone()),
        Box::new(MockMemory::default()),
        AgentRegistrySnapshot::default(),
        ToolPolicy::from_authority(&AgentAuthority {
            allow_command_execution: false,
            allow_file_reads: true,
            allow_file_writes: true,
            allow_file_search: true,
            allow_mcp: true,
            allow_agent_delegation: false,
            allow_notes: false,
            allow_text_responses: true,
            accessible_roots: vec![],
        }),
    ));

    let task = Task::new(AgentId::new(), "read startup.md");
    let _ = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 1,
            control: None,
        },
    ));

    let tools = reasoner.seen_tools().into_iter().next().unwrap_or_default();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"write_file"));
    assert!(!names.contains(&"run_command"));
    assert!(!names.contains(&"record_note"));
}

#[test]
fn spawn_agent_action_dispatches_through_local_agent_runtime() {
    let delegated = DelegatedTaskResult {
        agent_id: AgentId("local-child".to_string()),
        task_id: TaskId::new(),
        parent_task_id: None,
        status: DelegatedTaskStatus::Completed,
        summary: "child found the answer".to_string(),
        transcript_excerpt: Some(
            "action: read_file:/tmp/startup.md\nobserved: read file /tmp/startup.md".to_string(),
        ),
        output_path: None,
    };
    let local_runtime = MockLocalAgentRuntime {
        result: delegated.clone(),
        calls: Arc::new(Mutex::new(0)),
        routed_calls: Arc::new(Mutex::new(0)),
    };
    let kernel = must(Kernel::new_with_runtime(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::SpawnAgent {
                    id: ActionId::new(),
                    prompt: "Find the answer in startup.md".to_string(),
                    allowed_tools: vec!["read_file".to_string()],
                    denied_tools: vec!["run_command".to_string()],
                },
                task_complete: false,
                framing: None,
                reasoning: Some("delegate bounded reading work".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "child found the answer".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("integrate delegated result".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(MockMemory::default()),
        AgentRegistrySnapshot::default(),
        ToolPolicy::from_authority(&AgentAuthority::default()),
        Some(Arc::new(local_runtime.clone())),
        None,
    ));

    let outcome = must(kernel.execute_task(Task::new(AgentId::new(), "delegate this")));
    let Outcome::Success(ActionResult::Response { message }) = outcome else {
        panic!("expected final response");
    };
    assert_eq!(local_runtime.call_count(), 1);
    assert_eq!(message, "child found the answer");
}

#[test]
fn router_dispatches_existing_specialist_through_agent_runtime() {
    let delegated = DelegatedTaskResult {
        agent_id: AgentId("specialist-research".to_string()),
        task_id: TaskId::new(),
        parent_task_id: None,
        status: DelegatedTaskStatus::Completed,
        summary: "research specialist completed the task".to_string(),
        transcript_excerpt: Some(
            "action: list_directory:/tmp\nobserved: listed /tmp (4 entries)".to_string(),
        ),
        output_path: None,
    };
    let local_runtime = MockLocalAgentRuntime {
        result: delegated.clone(),
        calls: Arc::new(Mutex::new(0)),
        routed_calls: Arc::new(Mutex::new(0)),
    };
    let registry = AgentRegistrySnapshot {
        updated_at: chrono::Utc::now(),
        active_agents: vec![AgentCard {
            agent_id: AgentId("specialist-research".to_string()),
            domain: "research".to_string(),
            description: "research specialist".to_string(),
            capabilities: vec!["research".to_string(), "browser".to_string()],
            status: AgentStatus::Idle,
            lifecycle_phase: AgentLifecyclePhase::Ready,
            last_active_at: None,
        }],
        archived_agents: Vec::new(),
    };
    let kernel = must(Kernel::new_with_runtime(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "should not be used".to_string(),
        })),
        Box::new(MockMemory::default()),
        registry,
        ToolPolicy::from_authority(&AgentAuthority::default()),
        Some(Arc::new(local_runtime.clone())),
        None,
    ));

    let outcome = must(kernel.execute_task(Task::new(
        AgentId("root".to_string()),
        "research the browser support matrix",
    )));
    let Outcome::Success(ActionResult::DelegatedTask(result)) = outcome else {
        panic!("expected delegated task result");
    };
    assert_eq!(local_runtime.routed_call_count(), 1);
    assert_eq!(result.agent_id.0, "specialist-research");
}

#[test]
fn projected_task_state_keeps_authoritative_progress_without_advisory_frontier() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(4);
    state.seed_working_source(WorkingSource {
        locator: "startup.md".to_string(),
        kind: "file".to_string(),
        role: "authoritative".to_string(),
        status: "read".to_string(),
        why_it_matters: "source".to_string(),
        last_used_step: 2,
        evidence_refs: vec!["startup.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
        preview_excerpt: Some("startup preview".to_string()),
    });
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 2,
            kind: TranscriptUnitKind::ToolInvocation,
            summary: "read_file:startup.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read startup.md".to_string(),
            result_ref_id: None,
            primary_locator: Some("startup.md".to_string()),
            evidence_refs: vec!["startup.md".to_string()],
            working_sources: vec![WorkingSource {
                locator: "startup.md".to_string(),
                kind: "file".to_string(),
                role: "authoritative".to_string(),
                status: "read".to_string(),
                why_it_matters: "source".to_string(),
                last_used_step: 2,
                evidence_refs: vec!["startup.md".to_string()],
                page_reference: None,
                extraction_method: Some("text_read".to_string()),
                structured_summary: None,
                preview_excerpt: Some("startup preview".to_string()),
            }],
            artifact_references: vec![ArtifactReference {
                kind: "file".to_string(),
                locator: "startup.md".to_string(),
                status: "read".to_string(),
            }],
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "read startup.md and answer what it says");
    let task_state = kernel
        .build_active_continuation_window(&task, &state, 2, 4)
        .project_task_state();

    assert_eq!(task_state.working_sources.len(), 1);
    assert_eq!(task_state.working_sources[0].locator, "startup.md");
}

#[test]
fn continuation_window_carries_cached_read_state() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let task = Task::new(AgentId::new(), "inspect startup.md");
    let mut state = TaskLoopState::new(4);
    must(state.record_step(
        &task.id,
        &Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        },
        &Outcome::Success(ActionResult::FileRead {
            path: "startup.md".into(),
            content: "alpha\nbeta\n".to_string(),
            truncated: false,
            start_line: 1,
            line_count: 2,
            total_lines: 2,
            total_bytes: 11,
            read_bytes: 11,
        }),
    ));

    let window = kernel.build_active_continuation_window(&task, &state, 1, 4);
    assert_eq!(window.read_state_cache.len(), 1);
    assert_eq!(
        window.read_state_cache[0].path,
        std::path::PathBuf::from("startup.md")
    );
    assert!(!window.read_state_cache[0].was_partial);
    assert_eq!(window.read_state_cache[0].total_lines, 2);
}

#[test]
fn command_state_keeps_command_evidence_without_file_discovery_guidance() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(4);
    state.seed_working_source(WorkingSource {
        locator: "ps aux | grep -i docker | grep -v grep".to_string(),
        kind: "command".to_string(),
        role: "supporting".to_string(),
        status: "executed".to_string(),
        why_it_matters: "process check".to_string(),
        last_used_step: 2,
        evidence_refs: vec!["ps aux | grep -i docker | grep -v grep".to_string()],
        page_reference: None,
        extraction_method: Some("run_command".to_string()),
        structured_summary: None,
        preview_excerpt: Some("Docker Desktop still running".to_string()),
    });
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 2,
            kind: TranscriptUnitKind::ToolInvocation,
            summary: "run_command:ps aux | grep -i docker | grep -v grep".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "command succeeded with exit Some(0)".to_string(),
            result_ref_id: None,
            primary_locator: Some("ps aux | grep -i docker | grep -v grep".to_string()),
            evidence_refs: vec!["ps aux | grep -i docker | grep -v grep".to_string()],
            working_sources: vec![WorkingSource {
                locator: "ps aux | grep -i docker | grep -v grep".to_string(),
                kind: "command".to_string(),
                role: "supporting".to_string(),
                status: "executed".to_string(),
                why_it_matters: "process check".to_string(),
                last_used_step: 2,
                evidence_refs: vec!["ps aux | grep -i docker | grep -v grep".to_string()],
                page_reference: None,
                extraction_method: Some("run_command".to_string()),
                structured_summary: None,
                preview_excerpt: Some("Docker Desktop still running".to_string()),
            }],
            artifact_references: vec![ArtifactReference {
                kind: "command".to_string(),
                locator: "ps aux | grep -i docker | grep -v grep".to_string(),
                status: "executed".to_string(),
            }],
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "shutdown docker desktop");
    let task_state = kernel
        .build_active_continuation_window(&task, &state, 2, 4)
        .project_task_state();

    assert_eq!(task_state.working_sources.len(), 1);
    assert_eq!(
        task_state.working_sources[0].locator,
        "ps aux | grep -i docker | grep -v grep"
    );
}

#[test]
fn active_continuation_window_is_built_from_transcript_and_result_refs() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(6);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::TaskMessage,
            summary: "find startup.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read startup.md".to_string(),
            result_ref_id: Some("result-2-1".to_string()),
            primary_locator: Some("startup.md".to_string()),
            evidence_refs: vec!["startup.md".to_string()],
            working_sources: vec![WorkingSource {
                locator: "startup.md".to_string(),
                kind: "file".to_string(),
                role: "authoritative".to_string(),
                status: "read".to_string(),
                why_it_matters: "source".to_string(),
                last_used_step: 2,
                evidence_refs: vec!["startup.md".to_string()],
                page_reference: None,
                extraction_method: Some("text_read".to_string()),
                structured_summary: None,
                preview_excerpt: Some("hello from startup".to_string()),
            }],
            artifact_references: Vec::new(),
            next_step_guidance: Some(NextStepGuidance {
                directive: NextStepDirective::AnswerFromEvidence,
                reason: "the source was already read".to_string(),
                based_on_action: Some("read_file:startup.md".to_string()),
                evidence_locator: Some("startup.md".to_string()),
                preferred_search_family: None,
                suggested_query: None,
            }),
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);
    state.stored_results = StoredResultLedger::from_entries(vec![StoredResultReference {
        result_id: "result-2-1".to_string(),
        source_transcript_ordinal: 2,
        step: 2,
        result_type: "file_read".to_string(),
        primary_locator: Some("startup.md".to_string()),
        preview_excerpt: "hello from startup".to_string(),
        persisted_path: "/tmp/result-2-1.json".to_string(),
    }]);
    let task = Task::new(AgentId::new(), "read startup.md and answer what it says");
    let window = kernel.build_active_continuation_window(&task, &state, 3, 6);

    assert_eq!(window.objective, task.description);
    assert_eq!(window.transcript.len(), 3);
    assert!(window.transcript.entries().iter().any(|entry| {
        matches!(entry.kind, TranscriptUnitKind::CarryoverMessage)
            && entry.summary.contains("next-step guidance:")
    }));
    assert_eq!(window.stored_results.len(), 1);
    assert_eq!(
        window.stored_results.entries()[0]
            .primary_locator
            .as_deref(),
        Some("startup.md")
    );
    assert_eq!(
        window
            .next_step_guidance
            .as_ref()
            .and_then(|value| value.evidence_locator.as_deref()),
        Some("startup.md")
    );
}

#[test]
fn compaction_preserves_authoritative_sources_before_candidates() {
    let temp = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
    with_test_retina_home(&temp.path().join(".retina"), || {
        let task_id = TaskId::new();
        let mut state = TaskLoopState::new(8);
        for index in 0..8 {
            state.seed_working_source(WorkingSource {
                locator: format!("candidate-{index}.txt"),
                kind: "file".to_string(),
                role: "candidate".to_string(),
                status: "matched".to_string(),
                why_it_matters: "candidate".to_string(),
                last_used_step: index + 1,
                evidence_refs: vec![format!("candidate-{index}.txt")],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: None,
            });
        }
        state.seed_working_source(WorkingSource {
            locator: "authoritative.md".to_string(),
            kind: "file".to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "best source".to_string(),
            last_used_step: 4,
            evidence_refs: vec!["authoritative.md".to_string()],
            page_reference: None,
            extraction_method: Some("text_read".to_string()),
            structured_summary: None,
            preview_excerpt: Some("authoritative preview".to_string()),
        });
        let tool_results_dir = temp
            .path()
            .join(".retina")
            .join("root")
            .join("runtime")
            .join("tasks")
            .join(task_id.to_string())
            .join("tool-results");
        std::fs::create_dir_all(&tool_results_dir)
            .unwrap_or_else(|error| panic!("create_dir_all failed: {error}"));
        let stored_result_path = tool_results_dir.join("result-4-1.json");
        std::fs::write(
        &stored_result_path,
        "{\"type\":\"directory_listing\",\"root\":\"/Users/macc/Desktop\",\"count\":8,\"entries\":[],\"continuation\":\"use continuation candidate sources instead of replaying the full listing\"}",
    )
    .unwrap_or_else(|error| panic!("write failed: {error}"));
        state.stored_results = StoredResultLedger::from_entries(vec![StoredResultReference {
            result_id: "result-4-1".to_string(),
            source_transcript_ordinal: 8,
            step: 4,
            result_type: "directory_listing".to_string(),
            primary_locator: Some("/Users/macc/Desktop".to_string()),
            preview_excerpt: "listed many candidates".to_string(),
            persisted_path: stored_result_path.display().to_string(),
        }]);
        state.transcript = TranscriptLedger::from_entries(vec![
            TranscriptUnit {
                ordinal: 1,
                step: 1,
                kind: TranscriptUnitKind::ToolInvocation,
                summary: "list_directory:/Users/macc/Desktop".to_string(),
                result_ref_id: None,
                primary_locator: None,
                evidence_refs: Vec::new(),
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 2,
                step: 1,
                kind: TranscriptUnitKind::ToolResult,
                summary: "listed many candidates".to_string(),
                result_ref_id: None,
                primary_locator: Some("/Users/macc/Desktop".to_string()),
                evidence_refs: vec!["/Users/macc/Desktop".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 3,
                step: 2,
                kind: TranscriptUnitKind::ToolInvocation,
                summary: "find_files:/Users/macc/Desktop".to_string(),
                result_ref_id: None,
                primary_locator: None,
                evidence_refs: Vec::new(),
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 4,
                step: 2,
                kind: TranscriptUnitKind::ToolResult,
                summary: "matched candidate files".to_string(),
                result_ref_id: None,
                primary_locator: Some("candidate-1.txt".to_string()),
                evidence_refs: vec!["candidate-1.txt".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 5,
                step: 3,
                kind: TranscriptUnitKind::ToolInvocation,
                summary: "read_file:authoritative.md".to_string(),
                result_ref_id: None,
                primary_locator: None,
                evidence_refs: Vec::new(),
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 6,
                step: 3,
                kind: TranscriptUnitKind::ToolResult,
                summary: "read authoritative.md".to_string(),
                result_ref_id: None,
                primary_locator: Some("authoritative.md".to_string()),
                evidence_refs: vec!["authoritative.md".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 7,
                step: 4,
                kind: TranscriptUnitKind::ToolInvocation,
                summary: "search_text:authoritative".to_string(),
                result_ref_id: None,
                primary_locator: None,
                evidence_refs: Vec::new(),
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
            TranscriptUnit {
                ordinal: 8,
                step: 4,
                kind: TranscriptUnitKind::ToolResult,
                summary: "found authoritative mentions".to_string(),
                result_ref_id: None,
                primary_locator: Some("authoritative.md".to_string()),
                evidence_refs: vec!["authoritative.md".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            },
        ]);
        for index in 0..8 {
            state.seed_working_source(WorkingSource {
                locator: format!("candidate-{index}.txt"),
                kind: "file".to_string(),
                role: "candidate".to_string(),
                status: "matched".to_string(),
                why_it_matters: "candidate".to_string(),
                last_used_step: index + 1,
                evidence_refs: vec![format!("candidate-{index}.txt")],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: None,
            });
        }
        state.seed_working_source(WorkingSource {
            locator: "authoritative.md".to_string(),
            kind: "file".to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "best source".to_string(),
            last_used_step: 4,
            evidence_refs: vec!["authoritative.md".to_string()],
            page_reference: None,
            extraction_method: Some("text_read".to_string()),
            structured_summary: None,
            preview_excerpt: Some("authoritative preview".to_string()),
        });

        let decision = state.apply_live_compaction(&task_id);
        assert!(decision.is_some());
        assert!(
            state.working_sources().iter().any(
                |source| source.locator == "authoritative.md" && source.role == "authoritative"
            )
        );
        assert!(state.working_sources().len() <= 6);
        let snapshot = state
            .compaction_history()
            .last()
            .cloned()
            .expect("expected compaction snapshot");
        assert_eq!(snapshot.boundary_id, 1);
        assert_eq!(snapshot.compacted_at_step, 8);
        assert!(
            snapshot
                .preserved_locators
                .contains(&"authoritative.md".to_string())
        );
        assert!(!snapshot.active_window_summary.is_empty());
        assert_eq!(state.compaction_history().len(), 1);
        assert_eq!(snapshot.compacted_results.len(), 1);
        assert_eq!(snapshot.compacted_results[0].boundary_id, 1);
        assert_eq!(
            snapshot.compacted_results[0].result_type,
            "directory_listing"
        );
        assert_eq!(
            snapshot.compacted_results[0].locator.as_deref(),
            Some("/Users/macc/Desktop")
        );
        assert!(snapshot.compacted_results[0].persisted_path.is_some());
        assert!(
            state
                .transcript
                .entries()
                .iter()
                .any(|item| { matches!(item.kind, TranscriptUnitKind::CompactBoundary) })
        );
        assert!(
            state
                .transcript
                .entries()
                .iter()
                .any(|item| { matches!(item.kind, TranscriptUnitKind::CompactSummary) })
        );
        assert!(
            state
                .transcript
                .entries()
                .iter()
                .any(|item| { matches!(item.kind, TranscriptUnitKind::CompactedResultRef) })
        );
        assert!(
            state
                .transcript
                .entries()
                .iter()
                .any(|item| { matches!(item.kind, TranscriptUnitKind::MicroCompactBoundary) })
        );
        assert!(
            state
                .transcript
                .entries()
                .iter()
                .any(|item| { matches!(item.kind, TranscriptUnitKind::RestoredContinuation) })
        );
    });
}

#[test]
fn active_continuation_window_rebuilds_from_latest_compaction_boundary() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::TaskMessage,
            summary: "initial task".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "early result".to_string(),
            result_ref_id: Some("result-2-1".to_string()),
            primary_locator: Some("early.txt".to_string()),
            evidence_refs: vec!["early.txt".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 3,
            step: 3,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "step threshold".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: vec!["authoritative.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 4,
            step: 3,
            kind: TranscriptUnitKind::RestoredContinuation,
            summary: "continue from authoritative source".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: vec!["authoritative.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);
    state.stored_results = StoredResultLedger::from_entries(vec![StoredResultReference {
        result_id: "result-2-1".to_string(),
        source_transcript_ordinal: 2,
        step: 2,
        result_type: "file_read".to_string(),
        primary_locator: Some("early.txt".to_string()),
        preview_excerpt: "early preview".to_string(),
        persisted_path: "/tmp/result-2-1.json".to_string(),
    }]);
    state.seed_working_source(WorkingSource {
        locator: "early.txt".to_string(),
        kind: "file".to_string(),
        role: "candidate".to_string(),
        status: "read".to_string(),
        why_it_matters: "older".to_string(),
        last_used_step: 2,
        evidence_refs: vec!["early.txt".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
        preview_excerpt: Some("early preview".to_string()),
    });
    state.seed_working_source(WorkingSource {
        locator: "authoritative.md".to_string(),
        kind: "file".to_string(),
        role: "authoritative".to_string(),
        status: "read".to_string(),
        why_it_matters: "best source".to_string(),
        last_used_step: 3,
        evidence_refs: vec!["authoritative.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
        preview_excerpt: Some("authoritative preview".to_string()),
    });
    state.seed_artifact_reference(ArtifactReference {
        kind: "file".to_string(),
        locator: "authoritative.md".to_string(),
        status: "read".to_string(),
    });
    state.transcript = TranscriptLedger::from_entries(vec![TranscriptUnit {
        ordinal: 1,
        step: 3,
        kind: TranscriptUnitKind::CompactBoundary,
        summary: "step threshold".to_string(),
        result_ref_id: None,
        primary_locator: None,
        evidence_refs: vec!["authoritative.md".to_string()],
        working_sources: vec![WorkingSource {
            locator: "authoritative.md".to_string(),
            kind: "file".to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "best source".to_string(),
            last_used_step: 3,
            evidence_refs: vec!["authoritative.md".to_string()],
            page_reference: None,
            extraction_method: Some("text_read".to_string()),
            structured_summary: None,
            preview_excerpt: Some("authoritative preview".to_string()),
        }],
        artifact_references: vec![ArtifactReference {
            kind: "file".to_string(),
            locator: "authoritative.md".to_string(),
            status: "read".to_string(),
        }],
        next_step_guidance: None,
        repetition_signature: None,
        avoid_label: None,
        compaction_snapshot: Some(CompactionSnapshot {
            boundary_id: 1,
            compacted_at_step: 3,
            reason: "step threshold".to_string(),
            score_explanations: Vec::new(),
            preserved_locators: vec!["authoritative.md".to_string()],
            active_window_summary: "restored".to_string(),
            last_result_continuation: Some("continue from authoritative source".to_string()),
            compacted_results: Vec::new(),
        }),
    }]);

    let task = Task::new(AgentId::new(), "continue after compaction");
    let window = kernel.build_active_continuation_window(&task, &state, 4, 8);

    assert!(!window.transcript.entries().is_empty());
    assert!(
        !window
            .transcript
            .entries()
            .iter()
            .any(|item| item.summary == "early result")
    );
    assert!(window.transcript.entries().iter().any(|item| {
        matches!(item.kind, TranscriptUnitKind::CarryoverMessage)
            && item.summary.contains("source reminder:")
            && item.summary.contains("authoritative.md")
    }));
    assert_eq!(window.reannounced_sources[0].locator, "authoritative.md");
}

#[test]
fn active_continuation_window_starts_after_last_compact_boundary() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::TaskMessage,
            summary: "pre-boundary message".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "step threshold".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: Some(CompactionSnapshot {
                boundary_id: 1,
                compacted_at_step: 2,
                reason: "step threshold".to_string(),
                score_explanations: Vec::new(),
                preserved_locators: Vec::new(),
                active_window_summary: "boundary".to_string(),
                last_result_continuation: None,
                compacted_results: Vec::new(),
            }),
        },
        TranscriptUnit {
            ordinal: 3,
            step: 3,
            kind: TranscriptUnitKind::ToolInvocation,
            summary: "read_file:post.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 4,
            step: 3,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read post.md".to_string(),
            result_ref_id: None,
            primary_locator: Some("post.md".to_string()),
            evidence_refs: vec!["post.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "continue after compaction");
    let window = kernel.build_active_continuation_window(&task, &state, 3, 8);

    assert_eq!(
        window
            .transcript
            .entries()
            .iter()
            .map(|item| item.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TranscriptUnitKind::ToolInvocation,
            TranscriptUnitKind::ToolResult,
        ]
    );
    assert!(
        !window
            .transcript
            .entries()
            .iter()
            .any(|item| matches!(item.kind, TranscriptUnitKind::CompactBoundary))
    );
}

#[test]
fn active_continuation_window_keeps_only_latest_compaction_boundary() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "first compaction".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: Some(CompactionSnapshot {
                boundary_id: 1,
                compacted_at_step: 1,
                reason: "first compaction".to_string(),
                score_explanations: Vec::new(),
                preserved_locators: vec!["first.md".to_string()],
                active_window_summary: "first".to_string(),
                last_result_continuation: None,
                compacted_results: Vec::new(),
            }),
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "after first".to_string(),
            result_ref_id: None,
            primary_locator: Some("first.md".to_string()),
            evidence_refs: vec!["first.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 3,
            step: 3,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "second compaction".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: Some(CompactionSnapshot {
                boundary_id: 2,
                compacted_at_step: 3,
                reason: "second compaction".to_string(),
                score_explanations: Vec::new(),
                preserved_locators: vec!["second.md".to_string()],
                active_window_summary: "second".to_string(),
                last_result_continuation: None,
                compacted_results: Vec::new(),
            }),
        },
        TranscriptUnit {
            ordinal: 4,
            step: 4,
            kind: TranscriptUnitKind::ToolResult,
            summary: "after second".to_string(),
            result_ref_id: None,
            primary_locator: Some("second.md".to_string()),
            evidence_refs: vec!["second.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "continue after multiple compactions");
    let window = kernel.build_active_continuation_window(&task, &state, 4, 8);

    assert_eq!(window.compaction_boundaries.len(), 1);
    assert_eq!(window.compaction_boundaries[0].boundary_id, 2);
    assert_eq!(window.compaction_boundaries[0].reason, "second compaction");
}

#[test]
fn active_continuation_window_keeps_full_live_transcript_before_first_compaction() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(32);
    state.transcript = TranscriptLedger::from_entries(
        (0..15)
            .map(|index| TranscriptUnit {
                ordinal: index + 1,
                step: index + 1,
                kind: if index == 0 {
                    TranscriptUnitKind::TaskMessage
                } else {
                    TranscriptUnitKind::ToolResult
                },
                summary: format!("entry-{index}"),
                result_ref_id: None,
                primary_locator: None,
                evidence_refs: Vec::new(),
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            })
            .collect(),
    );

    let task = Task::new(AgentId::new(), "keep the full transcript before compaction");
    let window = kernel.build_active_continuation_window(&task, &state, 15, 32);

    assert_eq!(window.transcript.len(), 15);
    assert_eq!(
        window
            .transcript
            .entries()
            .first()
            .map(|item| item.summary.as_str()),
        Some("entry-0")
    );
    assert_eq!(
        window
            .transcript
            .entries()
            .last()
            .map(|item| item.summary.as_str()),
        Some("entry-14")
    );
}

#[test]
fn active_continuation_window_excludes_control_only_units_from_model_facing_transcript() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::TaskMessage,
            summary: "open chrome".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 1,
            kind: TranscriptUnitKind::ModelDecision,
            summary: "use browser tool".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 3,
            step: 1,
            kind: TranscriptUnitKind::OperatorGuidance,
            summary: "stay on validated path".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 4,
            step: 2,
            kind: TranscriptUnitKind::ToolInvocation,
            summary: "apps activate chrome".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 5,
            step: 2,
            kind: TranscriptUnitKind::GuidanceUpdate,
            summary: "prefer keyboard path".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 6,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "chrome activated".to_string(),
            result_ref_id: None,
            primary_locator: Some("Google Chrome".to_string()),
            evidence_refs: vec!["Google Chrome".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "open chrome");
    let window = kernel.build_active_continuation_window(&task, &state, 2, 8);

    assert_eq!(
        window
            .transcript
            .entries()
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TranscriptUnitKind::TaskMessage,
            TranscriptUnitKind::ToolInvocation,
            TranscriptUnitKind::ToolResult,
        ]
    );
}

#[test]
fn active_continuation_window_filters_control_only_transcript_units() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 0,
            kind: TranscriptUnitKind::TaskMessage,
            summary: "inspect startup.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 2,
            step: 1,
            kind: TranscriptUnitKind::ReflexDecision,
            summary: "matched read_file:startup.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 3,
            step: 1,
            kind: TranscriptUnitKind::CircuitBreakerState,
            summary: "failures=0 tripped=false".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 4,
            step: 1,
            kind: TranscriptUnitKind::ToolInvocation,
            summary: "read_file:startup.md".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
        TranscriptUnit {
            ordinal: 5,
            step: 1,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read startup.md".to_string(),
            result_ref_id: None,
            primary_locator: Some("startup.md".to_string()),
            evidence_refs: vec!["startup.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "inspect startup.md");
    let window = kernel.build_active_continuation_window(&task, &state, 1, 8);

    assert_eq!(window.transcript.len(), 3);
    assert_eq!(
        window
            .transcript
            .entries()
            .iter()
            .map(|item| item.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TranscriptUnitKind::TaskMessage,
            TranscriptUnitKind::ToolInvocation,
            TranscriptUnitKind::ToolResult
        ]
    );
}

#[test]
fn continuation_window_does_not_reannounce_source_already_covered_by_transcript() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "step threshold".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: vec![WorkingSource {
                locator: "other.md".to_string(),
                kind: "file".to_string(),
                role: "authoritative".to_string(),
                status: "listed".to_string(),
                why_it_matters: "generally important".to_string(),
                last_used_step: 1,
                evidence_refs: vec!["other.md".to_string()],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: None,
            }],
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: Some(CompactionSnapshot {
                boundary_id: 1,
                compacted_at_step: 1,
                reason: "step threshold".to_string(),
                score_explanations: Vec::new(),
                preserved_locators: vec!["other.md".to_string()],
                active_window_summary: "boundary".to_string(),
                last_result_continuation: None,
                compacted_results: Vec::new(),
            }),
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read candidate source".to_string(),
            result_ref_id: None,
            primary_locator: Some("candidate.md".to_string()),
            evidence_refs: vec!["candidate.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "continue from candidate source");
    let window = kernel.build_active_continuation_window(&task, &state, 2, 8);

    assert!(
        !window
            .reannounced_sources
            .iter()
            .any(|source| source.locator == "candidate.md")
    );
    assert!(
        window
            .reannounced_sources
            .iter()
            .any(|source| source.locator == "other.md")
    );
}

#[test]
fn continuation_window_does_not_reannounce_artifact_already_covered_by_transcript() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![
        TranscriptUnit {
            ordinal: 1,
            step: 1,
            kind: TranscriptUnitKind::CompactBoundary,
            summary: "step threshold".to_string(),
            result_ref_id: None,
            primary_locator: None,
            evidence_refs: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: vec![ArtifactReference {
                kind: "file".to_string(),
                locator: "other-artifact.md".to_string(),
                status: "created".to_string(),
            }],
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: Some(CompactionSnapshot {
                boundary_id: 1,
                compacted_at_step: 1,
                reason: "step threshold".to_string(),
                score_explanations: Vec::new(),
                preserved_locators: vec!["other-artifact.md".to_string()],
                active_window_summary: "boundary".to_string(),
                last_result_continuation: None,
                compacted_results: Vec::new(),
            }),
        },
        TranscriptUnit {
            ordinal: 2,
            step: 2,
            kind: TranscriptUnitKind::ToolResult,
            summary: "read artifact".to_string(),
            result_ref_id: None,
            primary_locator: Some("artifact.md".to_string()),
            evidence_refs: vec!["artifact.md".to_string()],
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            next_step_guidance: None,
            repetition_signature: None,
            avoid_label: None,
            compaction_snapshot: None,
        },
    ]);

    let task = Task::new(AgentId::new(), "continue from artifact");
    let window = kernel.build_active_continuation_window(&task, &state, 2, 8);

    assert!(
        !window
            .reannounced_artifacts
            .iter()
            .any(|artifact| artifact.locator == "artifact.md")
    );
    assert!(
        window
            .reannounced_artifacts
            .iter()
            .any(|artifact| artifact.locator == "other-artifact.md")
    );
}

#[test]
fn continuation_window_materializes_reannounced_context_into_transcript() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(8);
    state.transcript = TranscriptLedger::from_entries(vec![TranscriptUnit {
        ordinal: 1,
        step: 1,
        kind: TranscriptUnitKind::CompactBoundary,
        summary: "step threshold".to_string(),
        result_ref_id: None,
        primary_locator: None,
        evidence_refs: Vec::new(),
        working_sources: vec![WorkingSource {
            locator: "authoritative.md".to_string(),
            kind: "file".to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "carry forward".to_string(),
            last_used_step: 1,
            evidence_refs: vec!["authoritative.md".to_string()],
            page_reference: None,
            extraction_method: None,
            structured_summary: None,
            preview_excerpt: None,
        }],
        artifact_references: vec![ArtifactReference {
            kind: "file".to_string(),
            locator: "report.md".to_string(),
            status: "created".to_string(),
        }],
        next_step_guidance: Some(NextStepGuidance {
            directive: NextStepDirective::AnswerFromEvidence,
            reason: "continue with the preserved report".to_string(),
            based_on_action: None,
            evidence_locator: Some("report.md".to_string()),
            preferred_search_family: None,
            suggested_query: None,
        }),
        repetition_signature: None,
        avoid_label: None,
        compaction_snapshot: Some(CompactionSnapshot {
            boundary_id: 1,
            compacted_at_step: 1,
            reason: "step threshold".to_string(),
            score_explanations: Vec::new(),
            preserved_locators: vec!["authoritative.md".to_string(), "report.md".to_string()],
            active_window_summary: "boundary".to_string(),
            last_result_continuation: None,
            compacted_results: Vec::new(),
        }),
    }]);

    let task = Task::new(AgentId::new(), "continue after compaction");
    let window = kernel.build_active_continuation_window(&task, &state, 1, 8);
    let summaries = window
        .transcript
        .entries()
        .iter()
        .map(|entry| (entry.kind.clone(), entry.summary.clone()))
        .collect::<Vec<_>>();

    assert!(summaries.iter().any(|(kind, summary)| {
        matches!(kind, TranscriptUnitKind::CarryoverMessage)
            && summary.contains("source reminder:")
            && summary.contains("authoritative.md")
    }));
    assert!(summaries.iter().any(|(kind, summary)| {
        matches!(kind, TranscriptUnitKind::CarryoverMessage)
            && summary.contains("artifact reminder:")
            && summary.contains("report.md")
    }));
    assert!(summaries.iter().any(|(kind, summary)| {
        matches!(kind, TranscriptUnitKind::CarryoverMessage)
            && summary.contains("next-step guidance:")
            && summary.contains("continue with the preserved report")
    }));
}

#[test]
fn projected_task_state_stays_observational_without_inferred_deliverables() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let temp = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
    let target = temp.path().join("summary.md");
    std::fs::write(&target, "existing").unwrap_or_else(|error| panic!("write failed: {error}"));

    let mut state = TaskLoopState::new(6);
    state.seed_working_source(WorkingSource {
        locator: "startup.md".to_string(),
        kind: "file".to_string(),
        role: "authoritative".to_string(),
        status: "read".to_string(),
        why_it_matters: "source".to_string(),
        last_used_step: 3,
        evidence_refs: vec!["startup.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
        preview_excerpt: Some("preview".to_string()),
    });

    let objective = format!("update {} again from startup.md", target.display());
    let task = Task::new(AgentId::new(), &objective);
    let task_state = kernel
        .build_active_continuation_window(&task, &state, 3, 6)
        .project_task_state();
    assert_eq!(task_state.goal.objective, objective);
    assert_eq!(task_state.working_sources.len(), 1);
}

#[test]
fn output_flow_reaches_write_without_prompt_recovery_layer() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default().with_files([("startup.md", "hello")])),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::ReadFile {
                    id: ActionId::new(),
                    path: "startup.md".into(),
                    start_line: None,
                    limit_lines: None,
                    max_bytes: None,
                },
                task_complete: false,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("summary.md".to_string()),
                    completion_basis: None,
                }),
                reasoning: Some("read first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::FindFiles {
                    id: ActionId::new(),
                    root: ".".into(),
                    pattern: "*summary*".to_string(),
                    recursive: true,
                    max_results: 20,
                    offset: 0,
                },
                task_complete: false,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("summary.md".to_string()),
                    completion_basis: None,
                }),
                reasoning: Some("search again".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::WriteFile {
                    id: ActionId::new(),
                    path: "summary.md".into(),
                    content: "summary".to_string(),
                    overwrite: true,
                },
                task_complete: false,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("summary.md".to_string()),
                    completion_basis: Some("write the requested output".to_string()),
                }),
                reasoning: Some("write now".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("summary.md".to_string()),
                    completion_basis: Some("written and verified".to_string()),
                }),
                reasoning: Some("finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let task = Task::new(AgentId::new(), "create summary.md from startup.md");
    let outcome = must(kernel.execute_task_with_config(
        task.clone(),
        ExecutionConfig {
            max_steps: 4,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));

    let events = must(memory.recent_states(40));
    let dispatched = events
        .iter()
        .filter(|event| {
            event.task_id == task.id
                && matches!(event.event_type, TimelineEventType::ActionDispatched)
        })
        .map(|event| {
            event
                .payload_json
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(
        dispatched
            .iter()
            .any(|action| action.starts_with("write_file:summary.md"))
    );
}

#[test]
fn repeated_command_signature_groups_near_duplicate_process_checks() {
    let base_action = Action::RunCommand {
        id: ActionId::new(),
        command: "ps aux | grep -i docker | grep -v grep".to_string(),
        cwd: None,
        require_approval: false,
        expect_change: false,
        state_scope: HashScope::default(),
    };
    let variant_action = Action::RunCommand {
        id: ActionId::new(),
        command: "ps aux | grep -i docker | grep -v grep || echo 'No Docker processes found'"
            .to_string(),
        cwd: None,
        require_approval: false,
        expect_change: false,
        state_scope: HashScope::default(),
    };
    let head_variant_action = Action::RunCommand {
        id: ActionId::new(),
        command: "ps aux | grep -i docker | head -10".to_string(),
        cwd: None,
        require_approval: false,
        expect_change: false,
        state_scope: HashScope::default(),
    };
    let pgrep_variant_action = Action::RunCommand {
        id: ActionId::new(),
        command: "pgrep -f docker".to_string(),
        cwd: None,
        require_approval: false,
        expect_change: false,
        state_scope: HashScope::default(),
    };
    let result = ActionResult::Command(CommandResult {
        command: "ps aux | grep -i docker | grep -v grep".to_string(),
        cwd: ".".into(),
        stdout: "docker still running".to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
        duration_ms: 1,
        cancelled: false,
        termination: None,
        observed_paths: Vec::new(),
    });

    let base = crate::result_helpers::repeated_step_signature(&base_action, &result);
    let variant = crate::result_helpers::repeated_step_signature(&variant_action, &result);
    let head_variant =
        crate::result_helpers::repeated_step_signature(&head_variant_action, &result);
    let pgrep_variant =
        crate::result_helpers::repeated_step_signature(&pgrep_variant_action, &result);

    assert_eq!(base, variant);
    assert_eq!(base, head_variant);
    assert_eq!(base, pgrep_variant);
}

#[test]
fn repeated_mcp_portal_hit_escalates_to_limitation_guidance() {
    let mut state = TaskLoopState::new(6);
    let action = Action::CallMcpTool {
        id: ActionId::new(),
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        input_json: serde_json::json!({ "query": "denver events this weekend" }),
        resolved_tool_name: Some("mcp__brave__brave_web_search".to_string()),
    };
    let result = ActionResult::McpToolCall(McpToolCallResult {
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        content_preview: "{\"url\":\"https://visitdenver.com/blog/post/denver-events-this-weekend/\",\"title\":\"Denver Events & Things to Do This Weekend\"}".to_string(),
        structured_content: Some(serde_json::json!({
            "url": "https://visitdenver.com/blog/post/denver-events-this-weekend/",
            "title": "Denver Events & Things to Do This Weekend"
        })),
        is_error: false,
        search_outcome_kind: Some(McpSearchOutcomeKind::GenericPortal),
        evidence_identities: vec![
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            "Denver Events & Things to Do This Weekend".to_string(),
        ],
        search_hits: vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official Denver weekend events guide".to_string()),
        }],
        primary_locator: Some(
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ),
        evidence_summary: Some("Denver Events & Things to Do This Weekend: Official Denver weekend events guide".to_string()),
    });

    let task_id = TaskId::new();
    must(state.record_step(&task_id, &action, &Outcome::Success(result.clone())));
    let progress = must(state.record_step(&task_id, &action, &Outcome::Success(result)));
    assert!(progress.repeated_without_progress);

    let guidance = state
        .next_step_guidance()
        .clone()
        .expect("expected next-step guidance after repeated search hit");
    assert_eq!(guidance.directive, NextStepDirective::ReportLimitation);
    assert!(
        guidance
            .reason
            .contains("still broad portal-style listings")
    );
}

#[test]
fn verified_facts_include_directory_listing_counts_and_samples() {
    let facts = crate::result_helpers::summarize_verified_facts(
        &[WorkingSource {
            kind: "directory".to_string(),
            locator: "/Users/macc/Desktop/bulk-pdf".to_string(),
            role: "supporting".to_string(),
            status: "listed".to_string(),
            why_it_matters: "directory explored for task-relevant candidates".to_string(),
            last_used_step: 1,
            evidence_refs: vec!["/Users/macc/Desktop/bulk-pdf".to_string()],
            page_reference: None,
            extraction_method: None,
            structured_summary: None,
            preview_excerpt: Some(
                "4 entries (files=4, dirs=0); sample: ADV.pdf, Craig Lyons.pdf, Dominican_template.pdf, privacy policy.pdf".to_string(),
            ),
        }],
        &[],
    );

    assert!(facts.iter().any(|fact| fact.contains("4 entries")));
    assert!(facts.iter().any(|fact| fact.contains("privacy policy.pdf")));
}

#[test]
fn first_generic_portal_hit_suggests_narrower_search_family_and_query() {
    let mut state = TaskLoopState::new(6);
    let action = Action::CallMcpTool {
        id: ActionId::new(),
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        input_json: serde_json::json!({ "query": "denver events this weekend" }),
        resolved_tool_name: Some("mcp__brave__brave_web_search".to_string()),
    };
    let result = ActionResult::McpToolCall(McpToolCallResult {
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        content_preview: "{\"url\":\"https://visitdenver.com/blog/post/denver-events-this-weekend/\",\"title\":\"Denver Events & Things to Do This Weekend\"}".to_string(),
        structured_content: Some(serde_json::json!({
            "url": "https://visitdenver.com/blog/post/denver-events-this-weekend/",
            "title": "Denver Events & Things to Do This Weekend"
        })),
        is_error: false,
        search_outcome_kind: Some(McpSearchOutcomeKind::GenericPortal),
        evidence_identities: vec![
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            "Denver Events & Things to Do This Weekend".to_string(),
        ],
        search_hits: vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official Denver weekend events guide".to_string()),
        }],
        primary_locator: Some(
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ),
        evidence_summary: Some("Denver Events & Things to Do This Weekend: Official Denver weekend events guide".to_string()),
    });

    must(state.record_step(&TaskId::new(), &action, &Outcome::Success(result)));
    let guidance = state
        .next_step_guidance()
        .clone()
        .expect("expected next-step guidance after first search hit");
    assert_eq!(guidance.directive, NextStepDirective::ReformulateSearch);
    assert_eq!(
        guidance.preferred_search_family,
        Some(SearchToolFamily::News)
    );
    assert_eq!(
        guidance.suggested_query.as_deref(),
        Some("denver events this weekend specific dates venues tickets")
    );
}

#[test]
fn compact_mcp_search_result_carries_outcome_kind_and_identities() {
    let result = ActionResult::McpToolCall(McpToolCallResult {
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        content_preview: "{\"url\":\"https://visitdenver.com/blog/post/denver-events-this-weekend/\",\"title\":\"Denver Events & Things to Do This Weekend\"}".to_string(),
        structured_content: Some(serde_json::json!({
            "url": "https://visitdenver.com/blog/post/denver-events-this-weekend/",
            "title": "Denver Events & Things to Do This Weekend"
        })),
        is_error: false,
        search_outcome_kind: Some(McpSearchOutcomeKind::GenericPortal),
        evidence_identities: vec![
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            "Denver Events & Things to Do This Weekend".to_string(),
        ],
        search_hits: vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official Denver weekend events guide".to_string()),
        }],
        primary_locator: Some(
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ),
        evidence_summary: Some("Denver Events & Things to Do This Weekend: Official Denver weekend events guide".to_string()),
    });

    let compact = crate::result_helpers::compact_action_result_for_context(&result)
        .unwrap_or_else(|error| panic!("compact result failed: {error}"));

    assert!(compact.contains("\"search_outcome_kind\":\"generic_portal\""));
    assert!(compact.contains("\"evidence_identities\""));
    assert!(compact.contains("visitdenver.com"));
}

#[test]
fn mcp_working_source_uses_primary_locator_when_available() {
    let action = Action::CallMcpTool {
        id: ActionId::new(),
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        input_json: serde_json::json!({ "query": "denver events this weekend" }),
        resolved_tool_name: Some("mcp__brave__brave_web_search".to_string()),
    };
    let result = ActionResult::McpToolCall(McpToolCallResult {
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        content_preview: "Denver Events & Things to Do This Weekend".to_string(),
        structured_content: None,
        is_error: false,
        search_outcome_kind: Some(McpSearchOutcomeKind::GenericPortal),
        evidence_identities: vec![
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ],
        search_hits: vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official Denver weekend events guide".to_string()),
        }],
        primary_locator: Some(
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ),
        evidence_summary: Some("Denver Events & Things to Do This Weekend".to_string()),
    });
    let sources = crate::result_helpers::working_sources_for_result(&action, &result, 2);
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].locator,
        "https://visitdenver.com/blog/post/denver-events-this-weekend/"
    );
    assert!(
        sources[0]
            .evidence_refs
            .iter()
            .any(|value| value == "mcp-tool://brave/brave_web_search")
    );
}

#[test]
fn compacted_mcp_result_keeps_continuation_and_locator() {
    let result = ActionResult::McpToolCall(McpToolCallResult {
        server: "brave".to_string(),
        tool: "brave_web_search".to_string(),
        content_preview: "Denver weekend events preview".to_string(),
        structured_content: None,
        is_error: false,
        search_outcome_kind: Some(McpSearchOutcomeKind::GenericPortal),
        evidence_identities: vec![
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ],
        search_hits: vec![McpSearchHitSummary {
            url: "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
            title: Some("Denver Events & Things to Do This Weekend".to_string()),
            snippet: Some("Official Denver weekend events guide".to_string()),
        }],
        primary_locator: Some(
            "https://visitdenver.com/blog/post/denver-events-this-weekend/".to_string(),
        ),
        evidence_summary: Some("Denver Events & Things to Do This Weekend".to_string()),
    });

    let compact = crate::result_helpers::compact_last_result_for_compacted_context(
        &crate::result_helpers::compact_action_result_for_context(&result)
            .unwrap_or_else(|error| panic!("compact result failed: {error}")),
    )
    .unwrap_or_else(|error| panic!("microcompact failed: {error}"));

    assert!(compact.contains("\"type\":\"mcp_tool_call\""));
    assert!(compact.contains("\"primary_locator\""));
    assert!(compact.contains("\"continuation\""));
}

#[test]
fn directory_listing_sets_answer_from_evidence_guidance() {
    let temp = tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
    with_test_retina_home(&temp.path().join(".retina"), || {
        let mut state = TaskLoopState::new(4);
        let action = Action::ListDirectory {
            id: ActionId::new(),
            path: "/Users/macc/Desktop".into(),
            recursive: false,
            max_entries: 100,
        };
        let result = ActionResult::DirectoryListing {
            root: "/Users/macc/Desktop".into(),
            entries: vec![DirectoryEntry {
                path: "/Users/macc/Desktop/notes.txt".into(),
                is_dir: false,
                size: Some(10),
            }],
            summary: DirectoryListingSummary {
                total_entries: 1,
                file_count: 1,
                dir_count: 0,
                hidden_count: 0,
                sample_names: vec!["notes.txt".to_string()],
            },
        };

        must(state.record_step(&TaskId::new(), &action, &Outcome::Success(result)));

        let guidance = state
            .next_step_guidance()
            .clone()
            .expect("expected next-step guidance after directory listing");
        assert_eq!(guidance.directive, NextStepDirective::AnswerFromEvidence);
        assert_eq!(
            guidance.evidence_locator.as_deref(),
            Some("/Users/macc/Desktop")
        );
    });
}

#[test]
fn denied_approval_closes_operational_task_with_grounded_blocker() {
    let memory = MockMemory::default();
    let kernel = must(Kernel::new(
        Box::new(MockShell::default().with_approvals(vec![ApprovalResponse::Denied])),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "ps aux | grep -i docker | grep -v grep".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("check docker".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "sudo pkill -f docker".to_string(),
                    cwd: None,
                    require_approval: true,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("need stronger action".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(memory.clone()),
    ));

    let task = Task::new(AgentId::new(), "shut down docker desktop");
    let outcome = must(kernel.execute_task_with_config(
        task,
        ExecutionConfig {
            max_steps: 4,
            control: None,
        },
    ));

    match outcome {
        Outcome::Blocked(message) => {
            assert!(message.contains("requires approval and was denied"));
            assert!(message.contains("Earlier steps already attempted"));
            assert!(message.contains("Latest command evidence"));
        }
        _ => panic!("expected blocked outcome after denied approval"),
    }
}

#[test]
fn failed_step_stays_in_main_loop_and_can_finish_normally() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default().with_force_unchanged(true)),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::RunCommand {
                    id: ActionId::new(),
                    command: "generate".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: true,
                    state_scope: HashScope::default(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("try the command".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::ReadFile {
                    id: ActionId::new(),
                    path: "startup.md".into(),
                    start_line: None,
                    limit_lines: None,
                    max_bytes: None,
                },
                task_complete: true,
                framing: None,
                reasoning: Some("retry with a safer read".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Answer),
                    deliverable: Some("answer".to_string()),
                    completion_basis: Some("inspected startup.md and answered".to_string()),
                }),
                reasoning: Some("finish from gathered evidence".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "check startup.md"),
        ExecutionConfig {
            max_steps: 4,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
}

#[test]
fn non_response_task_complete_does_not_end_loop_early() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::ReadFile {
                    id: ActionId::new(),
                    path: "startup.md".into(),
                    start_line: None,
                    limit_lines: None,
                    max_bytes: None,
                },
                task_complete: true,
                framing: None,
                reasoning: Some("inspect first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: false,
                framing: None,
                reasoning: Some("only now finish".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "check startup.md"),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
}

#[test]
fn resumed_token_budget_block_preserves_recovery_transition() {
    let memory = MockMemory::default();
    let source_task = Task::new(AgentId::new(), "resume under token budget pressure");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id.clone(),
            source_session_id: source_task.session_id.clone(),
            source_agent_id: source_task.agent_id.clone(),
            objective: source_task.description.clone(),
            continuation_window: ActiveContinuationWindow {
                objective: source_task.description.clone(),
                current_step: 1,
                max_steps: 3,
                reasoner_tokens_used: 1000,
                max_output_tokens_recovery_count: 1,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: Some(ContinuationTransition {
                    reason: "max_output_tokens_recovery".to_string(),
                    attempt: Some(1),
                    message: Some(MAX_OUTPUT_TOKENS_RECOVERY_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after truncation recovery".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();
    let mut resumed = resumed;
    resumed
        .metadata
        .insert("max_tokens_per_task".to_string(), "1000".to_string());

    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_response(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "unused".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("unused".to_string()),
            tokens_used: TokenUsage::default(),
        })),
        Box::new(memory.clone()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(20));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("resumed token budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("max_output_tokens_recovery")
    );
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("max_output_tokens_recovery_count"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[test]
fn resumed_reasoner_call_budget_block_preserves_prompt_too_long_transition() {
    let memory = MockMemory::default();
    let source_task = Task::new(AgentId::new(), "resume under call budget pressure");
    let resumed = Task::resume_from_snapshot(
        AgentId::new(),
        TaskRecoverySnapshot {
            source_task_id: source_task.id.clone(),
            source_session_id: source_task.session_id.clone(),
            source_agent_id: source_task.agent_id.clone(),
            objective: source_task.description.clone(),
            continuation_window: ActiveContinuationWindow {
                objective: source_task.description.clone(),
                current_step: 2,
                max_steps: 3,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: true,
                last_transition: Some(ContinuationTransition {
                    reason: "prompt_too_long_compaction".to_string(),
                    attempt: Some(1),
                    message: Some(PROMPT_TOO_LONG_COMPACTION_MESSAGE.to_string()),
                    metadata: serde_json::Value::Null,
                }),
                transcript: TranscriptLedger::from_entries(vec![
                    TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: source_task.description.clone(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                    TranscriptUnit {
                        ordinal: 2,
                        step: 1,
                        kind: TranscriptUnitKind::ModelDecision,
                        summary: "prior planning decision".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    },
                ]),
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "continue after prompt-too-long compaction".to_string(),
        },
        None,
    );
    let resumed_task_id = resumed.id.clone();
    let mut resumed = resumed;
    resumed
        .metadata
        .insert("max_reasoner_calls_per_task".to_string(), "1".to_string());

    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_response(ReasonResponse {
            action: Action::Respond {
                id: ActionId::new(),
                message: "unused".to_string(),
            },
            task_complete: true,
            framing: None,
            reasoning: Some("unused".to_string()),
            tokens_used: TokenUsage::default(),
        })),
        Box::new(memory.clone()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        resumed,
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(matches!(outcome, Outcome::Blocked(_)));
    let events = must(memory.recent_states(20));
    let blocked = events
        .iter()
        .find(|event| {
            event.task_id == resumed_task_id
                && matches!(event.event_type, TimelineEventType::TaskBlocked)
        })
        .expect("resumed reasoner call budget block event should exist");
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("last_transition"))
            .and_then(|value| value.get("reason"))
            .and_then(|value| value.as_str()),
        Some("prompt_too_long_compaction")
    );
    assert_eq!(
        blocked
            .payload_json
            .get("continuation_window")
            .and_then(|value| value.get("has_attempted_prompt_too_long_compaction"))
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn file_write_task_complete_can_finish_via_tool_authored_completion() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![ReasonResponse {
            action: Action::WriteFile {
                id: ActionId::new(),
                path: "summary.md".into(),
                content: "saved summary".to_string(),
                overwrite: true,
            },
            task_complete: true,
            framing: Some(ReasonerTaskFraming {
                intent_kind: Some(TaskKind::Output),
                deliverable: Some("summary.md".to_string()),
                completion_basis: Some("saved requested output".to_string()),
            }),
            reasoning: Some("write the final artifact".to_string()),
            tokens_used: TokenUsage::default(),
        }])),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "create summary.md"),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    let Outcome::Success(ActionResult::Response { message }) = outcome else {
        panic!("expected tool-authored response, got {:?}", outcome);
    };
    assert!(message.contains("File created successfully at"));
    assert!(message.contains("summary.md"));
}

#[test]
fn generic_batch_deliverable_does_not_finish_via_single_file_write() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::WriteFile {
                    id: ActionId::new(),
                    path: "/Users/macc/Desktop/transcriptions/Craig Lyons - Summary.txt".into(),
                    content: "per-file summary".to_string(),
                    overwrite: true,
                },
                task_complete: true,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("transcriptions folder summary".to_string()),
                    completion_basis: Some("saved one summary file".to_string()),
                }),
                reasoning: Some("write one leaf artifact first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "still need the combined deliverable".to_string(),
                },
                task_complete: true,
                framing: Some(ReasonerTaskFraming {
                    intent_kind: Some(TaskKind::Output),
                    deliverable: Some("transcriptions folder summary".to_string()),
                    completion_basis: Some("reported remaining work".to_string()),
                }),
                reasoning: Some("single leaf file did not finish the batch".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(
            AgentId::new(),
            "summarize many PDFs and save the summary in the transcriptions folder",
        ),
        ExecutionConfig {
            max_steps: 3,
            control: None,
        },
    ));

    assert!(
        !matches!(
            &outcome,
            Outcome::Success(ActionResult::Response { message })
                if message.contains("File created successfully at")
        ),
        "single leaf write should not count as finishing the batch: {:?}",
        outcome
    );
}

#[test]
fn completion_blocker_detects_missing_batch_input_coverage() {
    let reason = crate::support::completion_blocker_reason(
        &Task::new(
            AgentId::new(),
            "read all the pdfs in desktop/bulk-pdf and save one combined report on desktop",
        ),
        &[
            WorkingSource {
                kind: "directory".to_string(),
                locator: "/Users/macc/Desktop/bulk-pdf".to_string(),
                role: "supporting".to_string(),
                status: "listed".to_string(),
                why_it_matters: "directory explored for task-relevant candidates".to_string(),
                last_used_step: 1,
                evidence_refs: vec![
                    "/Users/macc/Desktop/bulk-pdf".to_string(),
                    "/Users/macc/Desktop/bulk-pdf/ADV.pdf".to_string(),
                    "/Users/macc/Desktop/bulk-pdf/Craig Lyons.pdf".to_string(),
                    "/Users/macc/Desktop/bulk-pdf/Dominican_template.pdf".to_string(),
                    "/Users/macc/Desktop/bulk-pdf/privacy policy.pdf".to_string(),
                ],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: Some("4 entries (files=4, dirs=0)".to_string()),
            },
            WorkingSource {
                kind: "document".to_string(),
                locator: "/Users/macc/Desktop/bulk-pdf/ADV.pdf".to_string(),
                role: "authoritative".to_string(),
                status: "excerpted".to_string(),
                why_it_matters: "content source currently informing the task".to_string(),
                last_used_step: 2,
                evidence_refs: vec!["/Users/macc/Desktop/bulk-pdf/ADV.pdf".to_string()],
                page_reference: None,
                extraction_method: Some("pdf_extract_full".to_string()),
                structured_summary: None,
                preview_excerpt: Some("ADV excerpt".to_string()),
            },
            WorkingSource {
                kind: "document".to_string(),
                locator: "/Users/macc/Desktop/bulk-pdf/Craig Lyons.pdf".to_string(),
                role: "authoritative".to_string(),
                status: "excerpted".to_string(),
                why_it_matters: "content source currently informing the task".to_string(),
                last_used_step: 3,
                evidence_refs: vec!["/Users/macc/Desktop/bulk-pdf/Craig Lyons.pdf".to_string()],
                page_reference: None,
                extraction_method: Some("pdf_extract_full".to_string()),
                structured_summary: None,
                preview_excerpt: Some("resume excerpt".to_string()),
            },
            WorkingSource {
                kind: "document".to_string(),
                locator: "/Users/macc/Desktop/bulk-pdf/Dominican_template.pdf".to_string(),
                role: "authoritative".to_string(),
                status: "excerpted".to_string(),
                why_it_matters: "content source currently informing the task".to_string(),
                last_used_step: 4,
                evidence_refs: vec![
                    "/Users/macc/Desktop/bulk-pdf/Dominican_template.pdf".to_string(),
                ],
                page_reference: None,
                extraction_method: Some("pdf_extract_full".to_string()),
                structured_summary: None,
                preview_excerpt: Some("template excerpt".to_string()),
            },
        ],
    );

    let Some(reason) = reason else {
        panic!("expected batch completion blocker");
    };
    assert!(reason.contains("privacy policy.pdf"));
}

#[test]
fn explicit_response_is_the_normal_terminal_path() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "finished".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "say hi"),
        ExecutionConfig {
            max_steps: 2,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
}

#[test]
fn unsupported_document_read_remains_observable_in_main_loop() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::sequence(vec![
            ReasonResponse {
                action: Action::ExtractDocumentText {
                    id: ActionId::new(),
                    path: "Equipment Certificate.pdf".into(),
                    page_start: None,
                    page_end: None,
                    max_chars: None,
                },
                task_complete: false,
                framing: None,
                reasoning: Some("extract first".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::ReadFile {
                    id: ActionId::new(),
                    path: "Equipment Certificate.pdf".into(),
                    start_line: None,
                    limit_lines: None,
                    max_bytes: None,
                },
                task_complete: false,
                framing: None,
                reasoning: Some("try reading directly".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::ReadFile {
                    id: ActionId::new(),
                    path: "Equipment Certificate.pdf".into(),
                    start_line: None,
                    limit_lines: None,
                    max_bytes: None,
                },
                task_complete: false,
                framing: None,
                reasoning: Some("repeat the bad action".to_string()),
                tokens_used: TokenUsage::default(),
            },
            ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "done".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("answer once evidence exists".to_string()),
                tokens_used: TokenUsage::default(),
            },
        ])),
        Box::new(MockMemory::default()),
    ));

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "inspect equipment certificate"),
        ExecutionConfig {
            max_steps: 6,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));
}
impl MockLocalAgentRuntime {
    fn call_count(&self) -> usize {
        *recover_mutex(&self.calls)
    }

    fn routed_call_count(&self) -> usize {
        *recover_mutex(&self.routed_calls)
    }
}
