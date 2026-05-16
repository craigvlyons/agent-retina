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
fn initial_reasoner_context_includes_control_state_transcript_units() {
    let memory = MockMemory::default();
    must(memory.store_rule(&ReflexiveRule {
        id: Some(RuleId::new()),
        name: "startup reflex".to_string(),
        condition: RuleCondition::TaskContains("startup".to_string()),
        action: RuleAction::UseAction(Action::ReadFile {
            id: ActionId::new(),
            path: "startup.md".into(),
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
            max_steps: 2,
            control: None,
        },
    ));

    let seen = reasoner.seen_contexts();
    assert_eq!(seen.len(), 1);
    let transcript = seen[0].continuation_window.transcript.entries();
    assert!(transcript.iter().any(|item| {
        matches!(item.kind, TranscriptUnitKind::ReflexDecision)
            && item.summary.contains("matched read_file:startup.md")
    }));
    assert!(transcript.iter().any(|item| {
        matches!(item.kind, TranscriptUnitKind::CircuitBreakerState)
            && item.summary.contains("failures=0")
            && item.summary.contains("tripped=false")
    }));
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
                ..ActiveContinuationWindow::default()
            },
            recent_context: None,
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
    assert_eq!(window.transcript.len(), 2);
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

    assert_eq!(
        window.transcript.entries().first().map(|item| &item.kind),
        Some(&TranscriptUnitKind::CompactBoundary)
    );
    assert!(
        !window
            .transcript
            .entries()
            .iter()
            .any(|item| item.summary == "early result")
    );
    assert_eq!(window.reannounced_sources[0].locator, "authoritative.md");
}

#[test]
fn continuation_window_reannounces_transcript_referenced_source_even_if_not_preferred() {
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
            summary: "inspect candidate".to_string(),
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
    state.seed_working_source(WorkingSource {
        locator: "candidate.md".to_string(),
        kind: "file".to_string(),
        role: "candidate".to_string(),
        status: "read".to_string(),
        why_it_matters: "currently relevant".to_string(),
        last_used_step: 2,
        evidence_refs: vec!["candidate.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
        preview_excerpt: Some("candidate preview".to_string()),
    });
    state.seed_working_source(WorkingSource {
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
    });

    let task = Task::new(AgentId::new(), "continue from candidate source");
    let window = kernel.build_active_continuation_window(&task, &state, 2, 8);

    assert!(
        window
            .reannounced_sources
            .iter()
            .any(|source| source.locator == "candidate.md")
    );
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

    let outcome = must(kernel.execute_task_with_config(
        Task::new(AgentId::new(), "create summary.md from startup.md"),
        ExecutionConfig {
            max_steps: 4,
            control: None,
        },
    ));

    assert!(matches!(
        outcome,
        Outcome::Success(ActionResult::Response { .. })
    ));

    let events = must(memory.recent_states(20));
    let dispatched = events
        .iter()
        .filter(|event| matches!(event.event_type, TimelineEventType::ActionDispatched))
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
    assert!(
        !dispatched
            .iter()
            .any(|action| action.starts_with("find_files:.:*summary*:recursive=true"))
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

    let Outcome::Success(ActionResult::Response { message }) = outcome else {
        panic!("expected explicit follow-up response");
    };
    assert!(message.contains("combined deliverable"));
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
