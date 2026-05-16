use super::*;
use crate::support::recover_mutex;

use retina_test_utils::{MockMemory, MockReasoner, MockShell};
use retina_tools::ToolPolicy;
use retina_traits::{AgentRuntime, McpRuntime};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
    result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
}

#[derive(Clone)]
struct GuidanceReasoner {
    seen_task_states: Arc<Mutex<Vec<TaskState>>>,
    seen_tools: Arc<Mutex<Vec<Vec<ToolDescriptor>>>>,
    responses: Arc<Mutex<Vec<ReasonResponse>>>,
}

impl GuidanceReasoner {
    fn new(responses: Vec<ReasonResponse>) -> Self {
        Self {
            seen_task_states: Arc::new(Mutex::new(Vec::new())),
            seen_tools: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(responses)),
        }
    }

    fn seen_task_states(&self) -> Vec<TaskState> {
        recover_mutex(&self.seen_task_states).clone()
    }

    fn seen_tools(&self) -> Vec<Vec<ToolDescriptor>> {
        recover_mutex(&self.seen_tools).clone()
    }
}

impl retina_traits::Reasoner for GuidanceReasoner {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        recover_mutex(&self.seen_task_states).push(request.context.task_state.clone());
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
fn assembled_context_includes_structured_task_state() {
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

    let seen = reasoner.seen_task_states();
    assert!(!seen.is_empty());
    assert_eq!(
        seen[0].goal.objective,
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
    let seen = reasoner.seen_task_states();
    assert_eq!(seen.len(), 2);
    let last = seen[1]
        .recent_actions
        .last()
        .unwrap_or_else(|| panic!("expected recent failed action"));
    assert!(matches!(last.status, RecentActionStatus::Failed));
    assert!(last.outcome.contains("mcp-tool://brave/brave_web_search"));
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
fn task_state_keeps_authoritative_progress_without_advisory_frontier() {
    let kernel = must(Kernel::new(
        Box::new(MockShell::default()),
        Box::new(MockReasoner::for_action(Action::Respond {
            id: ActionId::new(),
            message: "done".to_string(),
        })),
        Box::new(MockMemory::default()),
    ));
    let mut state = TaskLoopState::new(4);
    state.step_index = 2;
    state.working_sources.push(WorkingSource {
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
    state.recent_action_summaries.push(RecentActionSummary {
        step: 2,
        action: "read_file:startup.md".to_string(),
        status: RecentActionStatus::Succeeded,
        outcome: "read startup.md".to_string(),
        artifact_refs: vec![ArtifactReference {
            kind: "file".to_string(),
            locator: "startup.md".to_string(),
            status: "read".to_string(),
        }],
    });
    state.artifact_references.push(ArtifactReference {
        kind: "file".to_string(),
        locator: "startup.md".to_string(),
        status: "read".to_string(),
    });

    let task = Task::new(AgentId::new(), "read startup.md and answer what it says");
    let task_state = kernel.build_task_state(&task, &state, 2, 4);

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
    state.step_index = 2;
    state.working_sources.push(WorkingSource {
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
    state.recent_action_summaries.push(RecentActionSummary {
        step: 2,
        action: "run_command:ps aux | grep -i docker | grep -v grep".to_string(),
        status: RecentActionStatus::Succeeded,
        outcome: "command succeeded with exit Some(0)".to_string(),
        artifact_refs: vec![ArtifactReference {
            kind: "command".to_string(),
            locator: "ps aux | grep -i docker | grep -v grep".to_string(),
            status: "executed".to_string(),
        }],
    });
    state.artifact_references.push(ArtifactReference {
        kind: "command".to_string(),
        locator: "ps aux | grep -i docker | grep -v grep".to_string(),
        status: "executed".to_string(),
    });

    let task = Task::new(AgentId::new(), "shutdown docker desktop");
    let task_state = kernel.build_task_state(&task, &state, 2, 4);

    assert_eq!(task_state.working_sources.len(), 1);
    assert_eq!(
        task_state.working_sources[0].locator,
        "ps aux | grep -i docker | grep -v grep"
    );
}

#[test]
fn compaction_preserves_authoritative_sources_before_candidates() {
    let mut state = TaskLoopState::new(8);
    state.step_index = 4;
    for index in 0..8 {
        state.working_sources.push(WorkingSource {
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
    state.working_sources.push(WorkingSource {
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
    state.last_result_json = Some("{\"type\":\"directory_listing\"}".to_string());
    state.last_result_summary = Some("listed many candidates".to_string());
    state.recent_steps = vec![
        "step 1".to_string(),
        "step 2".to_string(),
        "step 3".to_string(),
        "step 4".to_string(),
    ];

    let decision = state.apply_live_compaction();
    assert!(decision.is_some());
    assert!(
        state
            .working_sources
            .iter()
            .any(|source| source.locator == "authoritative.md" && source.role == "authoritative")
    );
    assert!(state.working_sources.len() <= 6);
}

#[test]
fn output_task_state_stays_observational_without_inferred_deliverables() {
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
    state.step_index = 3;
    state.working_sources.push(WorkingSource {
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
    let task_state = kernel.build_task_state(&task, &state, 3, 6);
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
        panic!("expected tool-authored response");
    };
    assert!(message.contains("File created successfully at"));
    assert!(message.contains("summary.md"));
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
