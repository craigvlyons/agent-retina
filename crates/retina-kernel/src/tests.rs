use super::*;
use crate::support::recover_mutex;

use retina_test_utils::{MockMemory, MockReasoner, MockShell};
use std::sync::{Arc, Mutex};

fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
    result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
}

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
    assert!(seen[0].intent_hint.is_none());
    assert!(
        seen[0]
            .goal
            .success_criteria
            .iter()
            .any(|item| item.contains("reduce the main unresolved obligation"))
    );
}

#[test]
fn task_state_frontier_prefers_authoritative_progress_over_generic_gap() {
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
    let task_state = kernel.build_task_state(&task, &state, 2, 4, Some("read startup.md".to_string()));

    assert_eq!(
        task_state.frontier.open_questions.first().map(String::as_str),
        Some("evidence gathered from authoritative sources")
    );
    assert_eq!(
        task_state.frontier.next_action_hint.as_deref(),
        Some("gathered evidence available for an answer")
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
    assert!(state
        .working_sources
        .iter()
        .any(|source| source.locator == "authoritative.md" && source.role == "authoritative"));
    assert!(state.working_sources.len() <= 6);
}

#[test]
fn reflection_retry_reenters_main_loop_and_can_finish_normally() {
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
fn unsupported_document_read_is_avoided_after_retry_feedback() {
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
