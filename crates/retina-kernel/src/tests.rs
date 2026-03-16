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


