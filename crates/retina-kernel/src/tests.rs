use super::*;
use crate::support::recover_mutex;
use crate::task_shape::completion_guard;

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
fn ungrounded_reasoner_completion_basis_is_blocked() {
    let mut task_state = TaskState::default();
    task_state.intent_hint = Some(TaskKind::Answer);
    task_state.reasoner_framing = Some(ReasonerTaskFraming {
        intent_kind: Some(TaskKind::Answer),
        deliverable: Some("summary".to_string()),
        completion_basis: Some("read startup.md and answered the question".to_string()),
    });
    task_state.recent_actions.push(RecentActionSummary {
        step: 1,
        action: "respond:done".to_string(),
        outcome: "responded to operator".to_string(),
        artifact_refs: Vec::new(),
    });

    let blocker = completion_guard(&task_state);
    assert!(matches!(
        blocker.as_deref(),
        Some("reasoner completion basis is not grounded in observed evidence yet")
    ));
}

#[test]
fn grounded_reasoner_completion_basis_passes_with_observed_evidence() {
    let mut task_state = TaskState::default();
    task_state.intent_hint = Some(TaskKind::Answer);
    task_state.reasoner_framing = Some(ReasonerTaskFraming {
        intent_kind: Some(TaskKind::Answer),
        deliverable: Some("summary".to_string()),
        completion_basis: Some("read startup.md and answered the question".to_string()),
    });
    task_state.working_sources.push(WorkingSource {
        locator: "startup.md".to_string(),
        kind: "file".to_string(),
        role: "authoritative".to_string(),
        status: "read".to_string(),
        why_it_matters: "source".to_string(),
        last_used_step: 1,
        evidence_refs: vec!["startup.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
    });
    task_state.recent_actions.push(RecentActionSummary {
        step: 2,
        action: "respond:done".to_string(),
        outcome: "responded to operator".to_string(),
        artifact_refs: Vec::new(),
    });

    assert!(completion_guard(&task_state).is_none());
}

#[test]
fn reasoner_intent_kind_overrides_heuristic_shape_for_completion() {
    let mut task_state = TaskState::default();
    task_state.reasoner_framing = Some(ReasonerTaskFraming {
        intent_kind: Some(TaskKind::Answer),
        deliverable: Some("summary".to_string()),
        completion_basis: Some("read startup.md and answered the question".to_string()),
    });
    task_state.working_sources.push(WorkingSource {
        locator: "startup.md".to_string(),
        kind: "file".to_string(),
        role: "authoritative".to_string(),
        status: "read".to_string(),
        why_it_matters: "source".to_string(),
        last_used_step: 1,
        evidence_refs: vec!["startup.md".to_string()],
        page_reference: None,
        extraction_method: Some("text_read".to_string()),
        structured_summary: None,
    });
    task_state.recent_actions.push(RecentActionSummary {
        step: 2,
        action: "respond:done".to_string(),
        outcome: "responded to operator".to_string(),
        artifact_refs: Vec::new(),
    });

    assert!(completion_guard(&task_state).is_none());
}

#[test]
fn discovery_only_completion_is_blocked_until_terminal_result_exists() {
    let mut task_state = TaskState::default();
    task_state.working_sources.push(WorkingSource {
        locator: "/Users/macc/Desktop".to_string(),
        kind: "directory".to_string(),
        role: "supporting".to_string(),
        status: "listed".to_string(),
        why_it_matters: "discovered initial location".to_string(),
        last_used_step: 1,
        evidence_refs: vec!["/Users/macc/Desktop".to_string()],
        page_reference: None,
        extraction_method: None,
        structured_summary: None,
    });
    task_state.recent_actions.push(RecentActionSummary {
        step: 1,
        action: "list_directory:/Users/macc/Desktop:recursive=false".to_string(),
        outcome: "listed desktop".to_string(),
        artifact_refs: vec![ArtifactReference {
            kind: "directory".to_string(),
            locator: "/Users/macc/Desktop".to_string(),
            status: "listed".to_string(),
        }],
    });

    let blocker = completion_guard(&task_state);
    assert!(matches!(
        blocker.as_deref(),
        Some(
            "task still needs a terminal result; intermediate shell steps must continue into a grounded response or verified output"
        )
    ));
}

#[test]
fn written_output_counts_as_terminal_result() {
    let mut task_state = TaskState::default();
    task_state.progress.output_written = true;
    task_state.progress.output_verified = true;
    task_state.recent_actions.push(RecentActionSummary {
        step: 2,
        action: "write_file:/tmp/out.txt".to_string(),
        outcome: "wrote output".to_string(),
        artifact_refs: vec![ArtifactReference {
            kind: "file".to_string(),
            locator: "/tmp/out.txt".to_string(),
            status: "written".to_string(),
        }],
    });

    assert!(completion_guard(&task_state).is_none());
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
        Some("Need to use authoritative evidence to finish the requested result")
    );
    assert_eq!(
        task_state.frontier.next_action_hint.as_deref(),
        Some(
            "Use the authoritative evidence to take the next verifiable synthesis, answer, or action step"
        )
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
