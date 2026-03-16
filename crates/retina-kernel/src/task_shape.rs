use super::TaskLoopState;
use retina_types::*;

pub(crate) fn describe_task_phase(
    state: &TaskLoopState,
    current_step: usize,
    max_steps: usize,
) -> String {
    if state.step_index == 0 {
        "starting".to_string()
    } else if current_step >= max_steps {
        "final step".to_string()
    } else {
        format!("working through step {} of {}", current_step, max_steps)
    }
}

pub(crate) fn current_intent_hint(state: &TaskLoopState) -> Option<TaskKind> {
    state.last_reasoner_framing.as_ref()?.intent_kind.clone()
}

pub(crate) fn build_task_frontier(
    last_result_summary: Option<String>,
    state: &TaskLoopState,
) -> (Vec<String>, Vec<String>, Option<String>) {
    let has_evidence = !state.working_sources.is_empty()
        || !state.artifact_references.is_empty()
        || !state.recent_action_summaries.is_empty();
    let mut open_questions = Vec::new();
    let blockers = Vec::new();

    if state.step_index > 0 && has_evidence {
        open_questions
            .push("Need to use the gathered evidence to finish the user's request".to_string());
    }

    let next_action_hint = if state.step_index > 0 && has_evidence {
        Some("Use the gathered evidence to make the next verifiable move or respond directly".to_string())
    } else {
        Some(match (last_result_summary, state.last_compaction_reason.as_ref()) {
            (Some(summary), Some(reason)) => format!(
                "Continue from compact task state ({reason}); use the latest verified result to choose the best next verifiable step toward the goal: {summary}"
            ),
            (Some(summary), None) => format!(
                "Use the latest verified result to choose the best next verifiable step toward the goal: {summary}"
            ),
            (None, Some(reason)) => format!("Continue from compact task state ({reason})"),
            (None, None) => "Choose the best next verifiable step from current task state".to_string(),
        })
    };

    (open_questions, blockers, next_action_hint)
}

pub(crate) fn completion_guard(task_state: &TaskState) -> Option<String> {
    if !completion_basis_is_grounded(task_state) {
        return Some(
            "reasoner completion basis is not grounded in observed evidence yet".to_string(),
        );
    }

    if task_state
        .recent_actions
        .last()
        .map(|summary| summary.action.starts_with("respond:"))
        .unwrap_or(false)
        && !task_state_has_grounded_answer(task_state)
    {
        return Some(
            "task still needs a grounded final answer, not just gathered evidence".to_string(),
        );
    }

    None
}

pub(crate) fn task_state_needs_terminal_result(task_state: &TaskState) -> bool {
    completion_guard(task_state).is_some()
}

fn task_state_has_grounded_answer(task_state: &TaskState) -> bool {
    let has_supporting_evidence = !task_state.working_sources.is_empty()
        || !task_state.artifact_references.is_empty()
        || !task_state.progress.verified_facts.is_empty();

    has_supporting_evidence
        && task_state
            .recent_actions
            .last()
            .map(|summary| summary.action.starts_with("respond:"))
            .unwrap_or(false)
}

fn completion_basis_is_grounded(task_state: &TaskState) -> bool {
    let Some(framing) = task_state.reasoner_framing.as_ref() else {
        return true;
    };
    let Some(basis) = framing.completion_basis.as_ref() else {
        return true;
    };
    let basis = basis.to_ascii_lowercase();
    let has_any_evidence = !task_state.working_sources.is_empty()
        || !task_state.artifact_references.is_empty()
        || !task_state.progress.verified_facts.is_empty();
    let has_output_evidence = task_state.artifact_references.iter().any(|artifact| {
        matches!(
            artifact.status.as_str(),
            "created" | "written" | "overwritten" | "appended" | "command_changed"
        )
    });
    let has_response = task_state
        .recent_actions
        .last()
        .map(|summary| summary.action.starts_with("respond:"))
        .unwrap_or(false);

    if contains_any(
        &basis,
        &[
            "write",
            "wrote",
            "created",
            "updated",
            "appended",
            "overwrote",
            "modified",
            "changed",
        ],
    ) {
        return has_output_evidence;
    }

    if basis.contains("respond") || basis.contains("answer") {
        return has_response && has_any_evidence;
    }

    if contains_any(
        &basis,
        &[
            "read",
            "ingested",
            "extracted",
            "searched",
            "found",
            "observed",
            "inspected",
            "listed",
        ],
    ) {
        return has_any_evidence;
    }

    has_any_evidence || has_output_evidence || has_response
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
