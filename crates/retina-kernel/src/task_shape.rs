use super::TaskLoopState;
use retina_types::*;
use std::path::{Path, PathBuf};

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

pub(crate) fn is_operational_command_task(task: &Task, state: &TaskLoopState) -> bool {
    if task_requests_output(task, state) {
        return false;
    }

    let lower = task.description.to_ascii_lowercase();
    let mentions_control = [
        "shutdown",
        "shut down",
        "stop",
        "start",
        "restart",
        "quit",
        "kill",
        "terminate",
        "close",
        "disable",
        "enable",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    mentions_control || has_command_state_evidence(state)
}

pub(crate) fn infer_target_output_path(task: &Task, state: &TaskLoopState) -> Option<String> {
    state
        .artifact_references
        .iter()
        .rev()
        .find(|artifact| {
            artifact.kind == "file"
                && matches!(
                    artifact.status.as_str(),
                    "created" | "written" | "overwritten" | "appended" | "command_changed"
                )
        })
        .map(|artifact| artifact.locator.clone())
        .or_else(|| parse_output_target_from_task(&task.description))
}

pub(crate) fn target_output_exists(
    target_output_path: Option<&str>,
    state: &TaskLoopState,
) -> bool {
    let Some(target_output_path) = target_output_path else {
        return false;
    };

    let resolved_target = resolve_user_facing_path(target_output_path);
    if resolved_target.as_ref().map(|path| path.exists()).unwrap_or(false) {
        return true;
    }

    state
        .artifact_references
        .iter()
        .any(|artifact| locator_matches_target(&artifact.locator, target_output_path))
        || state
            .working_sources
            .iter()
            .any(|source| locator_matches_target(&source.locator, target_output_path))
}

pub(crate) fn infer_pending_deliverable(
    task: &Task,
    state: &TaskLoopState,
    target_output_path: Option<&str>,
) -> Option<String> {
    target_output_path.map(|path| format!("write or update {}", display_path_hint(path))).or_else(
        || {
            state
                .last_reasoner_framing
                .as_ref()
                .and_then(|framing| framing.deliverable.clone())
                .or_else(|| {
                    task_requests_output(task, state)
                        .then(|| "produce the requested output artifact".to_string())
                })
        },
    )
}

pub(crate) fn infer_remaining_obligation(
    task: &Task,
    state: &TaskLoopState,
    output_written: bool,
    output_verified: bool,
    target_output_path: Option<&str>,
    target_output_exists: bool,
) -> Option<String> {
    let has_authoritative_source = has_authoritative_file_source(state);
    let has_generated_output = has_generated_artifact(state);
    let has_candidate_sources = has_file_candidate_sources(state);
    let has_command_state = has_command_state_evidence(state);

    match (
        task_requests_output(task, state),
        has_candidate_sources,
        has_authoritative_source,
        has_generated_output,
        output_written,
        output_verified,
        target_output_path,
    ) {
        (true, true, false, false, _, _, Some(path)) => Some(format!(
            "read the best source evidence before writing {}",
            display_path_hint(path)
        )),
        (true, _, true, false, _, _, Some(path)) if target_output_exists => Some(format!(
            "overwrite or confirm {} using the gathered evidence",
            display_path_hint(path)
        )),
        (true, _, true, false, _, _, Some(path)) => {
            Some(format!("write the requested output to {}", display_path_hint(path)))
        }
        (true, _, true, false, _, _, None) => {
            Some("turn the gathered evidence into the requested output".to_string())
        }
        (true, _, _, true, true, false, Some(path)) => Some(format!(
            "verify the written output at {} and then report completion",
            display_path_hint(path)
        )),
        (true, _, _, true, true, false, None) => {
            Some("verify the produced output and then report completion".to_string())
        }
        (true, _, _, true, _, true, _) => {
            Some("report the verified output back to the operator".to_string())
        }
        (false, true, false, false, _, _, _) => {
            Some("turn the best discovered candidate into authoritative evidence".to_string())
        }
        (false, _, true, false, _, _, _) => {
            Some("use the authoritative evidence to finish the requested answer".to_string())
        }
        (false, _, _, true, _, false, _) => {
            Some("verify the produced artifact or state change".to_string())
        }
        (false, _, false, false, _, _, _) if has_command_state => {
            Some("decide the next control action or report the current status".to_string())
        }
        _ => None,
    }
}

pub(crate) fn build_task_frontier(
    task: &Task,
    last_result_summary: Option<String>,
    state: &TaskLoopState,
    target_output_path: Option<&str>,
    target_output_exists: bool,
    remaining_obligation: Option<&str>,
) -> (Vec<String>, Vec<String>, Option<String>) {
    let has_evidence = !state.working_sources.is_empty()
        || !state.artifact_references.is_empty()
        || !state.recent_action_summaries.is_empty();
    let has_authoritative_source = has_authoritative_file_source(state);
    let has_generated_output = has_generated_artifact(state);
    let has_candidate_sources = has_file_candidate_sources(state);
    let has_command_state = has_command_state_evidence(state);
    let mut open_questions = Vec::new();
    let mut blockers = Vec::new();
    if state.step_index > 0 && has_evidence {
        if let Some(obligation) = remaining_obligation {
            open_questions.push(obligation.to_string());
        } else if has_command_state && !has_authoritative_source && !has_generated_output {
            open_questions.push("command-state evidence gathered".to_string());
        } else if has_candidate_sources && !has_authoritative_source && !has_generated_output {
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
        Some(if task_requests_output(task, state) {
            match (has_candidate_sources, has_authoritative_source, has_generated_output, target_output_path) {
                (true, false, false, Some(path)) => format!(
                    "candidate source available; inspect or read the best source before writing {}",
                    display_path_hint(path)
                ),
                (_, true, false, Some(path)) if target_output_exists => format!(
                    "authoritative evidence ready; overwrite or confirm {} instead of searching again",
                    display_path_hint(path)
                ),
                (_, true, false, Some(path)) => format!(
                    "authoritative evidence ready; write the requested output to {}",
                    display_path_hint(path)
                ),
                (_, true, false, None) => {
                    "authoritative evidence ready; create the requested output artifact".to_string()
                }
                (_, _, true, Some(path)) => format!(
                    "output artifact present; verify {} and then report completion",
                    display_path_hint(path)
                ),
                (_, _, true, None) => {
                    "produced artifact present; verify it and then report completion".to_string()
                }
                _ if has_command_state => {
                    "latest command result available; choose the next control step or report status".to_string()
                }
                _ => "latest observed result available for the next completion step".to_string(),
            }
        } else if has_command_state && !has_candidate_sources && !has_authoritative_source && !has_generated_output {
            if blockers.is_empty() {
                "latest command result available; choose the next control step or report status".to_string()
            } else {
                "latest command result available; choose a materially different control step or report blocker".to_string()
            }
        } else if has_candidate_sources && !has_authoritative_source && !has_generated_output {
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

pub(crate) fn should_reconsider_low_value_action(
    task: &Task,
    state: &TaskLoopState,
    action: &Action,
    target_output_path: Option<&str>,
) -> Option<String> {
    if task_requests_output(task, state) {
        if !state
            .working_sources
            .iter()
            .any(|source| source.role == "authoritative")
        {
            return None;
        }
        let target_output_path = target_output_path?;
        let is_low_value = match action {
            Action::FindFiles { .. }
            | Action::SearchText { .. }
            | Action::ListDirectory { .. }
            | Action::InspectWorkingDirectory { .. } => true,
            Action::InspectPath { path, .. } => !path_matches_target(path, target_output_path),
            _ => false,
        };
        if !is_low_value {
            return None;
        }

        return Some(format!(
            "authoritative evidence and the named output target {} are already available; prefer writing, verifying, or responding instead of broad discovery",
            display_path_hint(target_output_path)
        ));
    }

    if !is_operational_command_task(task, state) {
        return None;
    }

    let action_label = match action {
        Action::RunCommand { command, .. } => command.as_str(),
        _ => return None,
    };
    let already_tried = state
        .recent_action_summaries
        .iter()
        .rev()
        .any(|summary| summary.action == format!("run_command:{action_label}"));
    if !already_tried {
        return None;
    }

    Some(
        "that control step already ran and the remaining blocker is still unresolved; choose a materially different action or report the grounded current status/blocker".to_string(),
    )
}

fn task_requests_output(task: &Task, state: &TaskLoopState) -> bool {
    if state
        .last_reasoner_framing
        .as_ref()
        .and_then(|framing| framing.intent_kind.as_ref())
        .is_some_and(|kind| matches!(kind, TaskKind::Output))
    {
        return true;
    }

    let lower = task.description.to_lowercase();
    [
        "create ",
        "write ",
        "save ",
        "update ",
        "rewrite ",
        "overwrite ",
        "append ",
        "generate ",
        "make ",
        "export ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn has_authoritative_file_source(state: &TaskLoopState) -> bool {
    state
        .working_sources
        .iter()
        .any(|source| source.kind != "command" && source.role == "authoritative")
}

fn has_generated_artifact(state: &TaskLoopState) -> bool {
    state.working_sources.iter().any(|source| source.role == "generated")
        || state.artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "created" | "written" | "overwritten" | "appended" | "command_changed"
            )
        })
}

fn has_file_candidate_sources(state: &TaskLoopState) -> bool {
    state
        .working_sources
        .iter()
        .any(|source| source.kind != "command" && matches!(source.role.as_str(), "candidate" | "supporting"))
        || state.artifact_references.iter().any(|artifact| {
            artifact.kind != "command"
                && matches!(artifact.status.as_str(), "matched" | "listed" | "inspected")
        })
}

fn has_command_state_evidence(state: &TaskLoopState) -> bool {
    state
        .working_sources
        .iter()
        .any(|source| source.kind == "command" && source.status == "executed")
}


fn parse_output_target_from_task(task: &str) -> Option<String> {
    let tokens = task
        .split_whitespace()
        .map(clean_task_token)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut best: Option<(usize, String)> = None;

    for (index, token) in tokens.iter().enumerate() {
        if !looks_like_file_target(token) {
            continue;
        }

        let lower_token = token.to_ascii_lowercase();
        let mut score = 100usize;
        let window_start = index.saturating_sub(4);
        let prior_window = tokens[window_start..index]
            .iter()
            .map(|item| item.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let next_window = tokens
            .iter()
            .skip(index + 1)
            .take(4)
            .map(|item| item.to_ascii_lowercase())
            .collect::<Vec<_>>();

        if token.contains('/') || token.starts_with('~') {
            score = score.saturating_sub(25);
        }
        if prior_window.iter().any(|item| is_output_verb(item)) {
            score = score.saturating_sub(40);
        }
        if next_window.iter().any(|item| item == "desktop" || item == "documents" || item == "downloads")
        {
            score = score.saturating_sub(20);
        }
        if prior_window.iter().any(|item| item == "desktop" || item == "documents" || item == "downloads")
        {
            score = score.saturating_sub(20);
        }
        if prior_window
            .iter()
            .any(|item| matches!(item.as_str(), "read" | "summarize" | "from" | "use"))
        {
            score += 20;
        }

        let candidate = if token.contains('/') || token.starts_with('~') {
            token.clone()
        } else if prior_window.iter().any(|item| item == "desktop")
            || next_window.iter().any(|item| item == "desktop")
        {
            format!("~/Desktop/{token}")
        } else if prior_window.iter().any(|item| item == "documents")
            || next_window.iter().any(|item| item == "documents")
        {
            format!("~/Documents/{token}")
        } else if prior_window.iter().any(|item| item == "downloads")
            || next_window.iter().any(|item| item == "downloads")
        {
            format!("~/Downloads/{token}")
        } else {
            token.clone()
        };

        if best.as_ref().map(|(best_score, _)| score < *best_score).unwrap_or(true) {
            best = Some((score, candidate));
        }

        if lower_token.contains(".md") || lower_token.contains(".txt") {
            best = best.map(|(_, value)| (score.saturating_sub(5), value));
        }
    }

    best.map(|(_, value)| value)
}

fn clean_task_token(token: &str) -> String {
    token
        .trim_matches(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~')))
        .trim_end_matches('.')
        .to_string()
}

fn looks_like_file_target(token: &str) -> bool {
    let Some((_, extension)) = token.rsplit_once('.') else {
        return false;
    };
    extension.chars().all(|c| c.is_ascii_alphanumeric()) && !extension.is_empty()
}

fn is_output_verb(token: &str) -> bool {
    matches!(
        token,
        "create"
            | "write"
            | "save"
            | "update"
            | "rewrite"
            | "overwrite"
            | "append"
            | "generate"
            | "make"
            | "export"
    )
}

fn display_path_hint(path: &str) -> String {
    resolve_user_facing_path(path)
        .map(|resolved| resolved.display().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn path_matches_target(path: &Path, target_output_path: &str) -> bool {
    locator_matches_target(&path.display().to_string(), target_output_path)
}

fn locator_matches_target(locator: &str, target_output_path: &str) -> bool {
    let resolved_locator = resolve_user_facing_path(locator);
    let resolved_target = resolve_user_facing_path(target_output_path);
    if let (Some(locator), Some(target)) = (resolved_locator.as_ref(), resolved_target.as_ref()) {
        if locator == target {
            return true;
        }
    }

    let locator_name = resolved_locator
        .as_ref()
        .and_then(|path| path.file_name().and_then(|value| value.to_str()))
        .or_else(|| Path::new(locator).file_name().and_then(|value| value.to_str()));
    let target_name = resolved_target
        .as_ref()
        .and_then(|path| path.file_name().and_then(|value| value.to_str()))
        .or_else(|| Path::new(target_output_path).file_name().and_then(|value| value.to_str()));
    locator_name == target_name
}

fn resolve_user_facing_path(raw: &str) -> Option<PathBuf> {
    if raw == "~" {
        return dirs::home_dir();
    }
    if let Some(stripped) = raw.strip_prefix("~/") {
        let stripped_path = Path::new(stripped);
        if let Some(alias) = resolve_known_folder_alias(stripped_path) {
            return Some(alias);
        }
        return dirs::home_dir().map(|home| home.join(stripped));
    }
    if raw.starts_with('/') {
        return Some(PathBuf::from(raw));
    }
    resolve_known_folder_alias(Path::new(raw))
}

fn resolve_known_folder_alias(path: &Path) -> Option<PathBuf> {
    let first = path
        .components()
        .next()?
        .as_os_str()
        .to_str()?
        .to_ascii_lowercase();
    let base = match first.as_str() {
        "desktop" => dirs::desktop_dir(),
        "documents" => dirs::document_dir(),
        "downloads" => dirs::download_dir(),
        _ => None,
    }?;
    let remainder = path.iter().skip(1).collect::<PathBuf>();
    Some(if remainder.as_os_str().is_empty() {
        base
    } else {
        base.join(remainder)
    })
}
