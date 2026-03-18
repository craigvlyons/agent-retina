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
    let has_authoritative_source = state
        .working_sources
        .iter()
        .any(|source| source.role == "authoritative");
    let has_generated_output = state.working_sources.iter().any(|source| source.role == "generated")
        || state.artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "created" | "written" | "overwritten" | "appended" | "command_changed"
            )
        });
    let has_candidate_sources = state
        .working_sources
        .iter()
        .any(|source| matches!(source.role.as_str(), "candidate" | "supporting"))
        || state
            .artifact_references
            .iter()
            .any(|artifact| matches!(artifact.status.as_str(), "matched" | "listed" | "inspected"));
    let mut open_questions = Vec::new();
    let mut blockers = Vec::new();

    if state.step_index > 0 && has_evidence {
        if has_candidate_sources && !has_authoritative_source && !has_generated_output {
            open_questions.push("candidate paths observed".to_string());
        } else if has_authoritative_source && !has_generated_output {
            open_questions.push("evidence gathered from authoritative sources".to_string());
        } else if has_generated_output {
            open_questions.push("artifact change observed".to_string());
        } else {
            open_questions.push("evidence gathered for the request".to_string());
        }
    }

    if let Some(avoid) = state.avoid_rules.last() {
        blockers.push(format!("avoid repeating {} because {}", avoid.label, avoid.reason));
    }

    let next_action_hint = if state.step_index > 0 && has_evidence {
        Some(if has_candidate_sources && !has_authoritative_source && !has_generated_output {
            "candidate path available for inspection or reading".to_string()
        } else if has_authoritative_source && !has_generated_output {
            "gathered evidence available for an answer".to_string()
        } else if has_generated_output {
            "produced artifact or world change available for verification".to_string()
        } else {
            "latest observed result available for the next step".to_string()
        })
    } else {
        Some(match (last_result_summary, state.last_compaction_reason.as_ref()) {
            (Some(summary), Some(reason)) => format!(
                "compact state preserved ({reason}); latest observed result: {summary}"
            ),
            (Some(summary), None) => format!("latest observed result: {summary}"),
            (None, Some(reason)) => format!("Continue from compact task state ({reason})"),
            (None, None) => "current task state available".to_string(),
        })
    };

    (open_questions, blockers, next_action_hint)
}

