use super::TaskLoopState;
use retina_types::*;
use std::collections::HashSet;
use std::path::Path;

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

pub(crate) fn infer_task_shape(task_description: &str, state: &TaskLoopState) -> TaskShape {
    let file_mentions = file_mentions_with_positions(task_description);
    let requested_output = infer_requested_output(task_description, &file_mentions, state);
    let output_hint = requested_output
        .as_ref()
        .map(|output| normalize_locator(&output.locator_hint));

    let required_inputs = file_mentions
        .into_iter()
        .scan(HashSet::new(), |seen, (_, hint)| {
            let normalized = normalize_locator(&hint);
            if !seen.insert(normalized) {
                return Some(None);
            }
            Some(Some(hint))
        })
        .flatten()
        .filter_map(|hint| {
            let normalized = normalize_locator(&hint);
            if output_hint
                .as_ref()
                .is_some_and(|output| output == &normalized)
            {
                return None;
            }
            Some(RequiredInput {
                kind: classify_locator_kind(&hint),
                status: infer_required_input_status(&hint, state),
                locator_hint: hint,
            })
        })
        .collect::<Vec<_>>();

    let kind = infer_task_kind(
        task_description,
        requested_output.as_ref(),
        &required_inputs,
    );
    let success_markers =
        build_success_markers(kind.clone(), &required_inputs, requested_output.as_ref());

    TaskShape {
        kind,
        required_inputs,
        requested_output,
        success_markers,
    }
}

pub(crate) fn build_task_frontier(
    shape: &TaskShape,
    last_result_summary: Option<String>,
    state: &TaskLoopState,
) -> (Vec<String>, Vec<String>, Option<String>) {
    let unresolved_inputs = shape
        .required_inputs
        .iter()
        .filter(|input| !required_input_is_satisfied(input))
        .map(|input| input.locator_hint.clone())
        .collect::<Vec<_>>();

    let mut open_questions = unresolved_inputs
        .iter()
        .map(|hint| format!("Need to locate and ingest {hint}"))
        .collect::<Vec<_>>();
    let mut blockers = Vec::new();

    if matches!(shape.kind, TaskKind::Transform | TaskKind::Output) && !unresolved_inputs.is_empty()
    {
        blockers.extend(
            unresolved_inputs
                .iter()
                .map(|hint| format!("required input still not ingested: {hint}")),
        );
    }

    if matches!(shape.kind, TaskKind::Answer) && !unresolved_inputs.is_empty() {
        blockers.extend(unresolved_inputs.iter().map(|hint| {
            format!("need evidence from {hint} before a grounded answer can be returned")
        }));
    }

    if let Some(output) = shape
        .requested_output
        .as_ref()
        .filter(|output| !output.verified)
    {
        open_questions.push(format!("Need to create and verify {}", output.locator_hint));
        if matches!(shape.kind, TaskKind::Output | TaskKind::Transform)
            && unresolved_inputs.is_empty()
            && state.step_index > 0
        {
            blockers.push(format!(
                "task is not complete until {} exists as a verified output artifact",
                output.locator_hint
            ));
        }
    }

    if matches!(shape.kind, TaskKind::Answer)
        && unresolved_inputs.is_empty()
        && state.step_index > 0
    {
        open_questions
            .push("Need to turn the gathered evidence into a grounded final answer".to_string());
        if !state_has_terminal_result(state, shape) {
            blockers
                .push("task is not complete until a grounded final answer is returned".to_string());
        }
    }

    if matches!(shape.kind, TaskKind::Transform)
        && shape.requested_output.is_none()
        && unresolved_inputs.is_empty()
        && state.step_index > 0
    {
        open_questions.push(
            "Need to synthesize the ingested sources into the requested transformed result"
                .to_string(),
        );
        if !state_has_terminal_result(state, shape) {
            blockers.push(
                "task is not complete until the ingested evidence is synthesized into a final result"
                    .to_string(),
            );
        }
    }

    let next_action_hint = if !unresolved_inputs.is_empty() {
        Some(format!(
            "Locate or ingest remaining required inputs: {}",
            unresolved_inputs.join(", ")
        ))
    } else if matches!(shape.kind, TaskKind::Answer) && state.step_index > 0 {
        Some("Return a grounded final answer from the ingested evidence".to_string())
    } else if let Some(output) = shape
        .requested_output
        .as_ref()
        .filter(|output| !output.verified)
    {
        Some(format!(
            "Create and verify the requested output artifact: {}",
            output.locator_hint
        ))
    } else if matches!(shape.kind, TaskKind::Transform) && state.step_index > 0 {
        Some("Synthesize the ingested evidence into the requested transformed result".to_string())
    } else {
        Some(
            match (last_result_summary, state.last_compaction_reason.as_ref()) {
                (Some(summary), Some(reason)) => format!(
                    "Continue from compact task state ({reason}); use the latest verified result to choose the best next verifiable step toward the goal: {summary}"
                ),
                (Some(summary), None) => format!(
                    "Use the latest verified result to choose the best next verifiable step toward the goal: {summary}"
                ),
                (None, Some(reason)) => format!("Continue from compact task state ({reason})"),
                (None, None) => {
                    "Choose the best next verifiable step from current task state".to_string()
                }
            },
        )
    };

    (open_questions, blockers, next_action_hint)
}

pub(crate) fn completion_guard(task_state: &TaskState) -> Option<String> {
    if !matches!(
        task_state.shape.kind,
        TaskKind::Answer | TaskKind::Transform | TaskKind::Output
    ) {
        return None;
    }

    let unresolved_inputs = task_state
        .shape
        .required_inputs
        .iter()
        .filter(|input| !required_input_is_satisfied(input))
        .map(|input| input.locator_hint.clone())
        .collect::<Vec<_>>();
    if !unresolved_inputs.is_empty() {
        return Some(format!(
            "required inputs are not fully ingested yet: {}",
            unresolved_inputs.join(", ")
        ));
    }

    if let Some(output) = task_state.shape.requested_output.as_ref() {
        if !output.verified {
            return Some(format!(
                "requested output is not verified yet: {}",
                output.locator_hint
            ));
        }
    }

    if matches!(
        task_state.shape.kind,
        TaskKind::Answer | TaskKind::Transform
    ) && !task_state_has_terminal_result(task_state)
    {
        return Some(match task_state.shape.kind {
            TaskKind::Answer => {
                "task still needs a grounded final answer, not just gathered evidence".to_string()
            }
            TaskKind::Transform => {
                "task still needs the gathered evidence to be synthesized into a final result"
                    .to_string()
            }
            _ => unreachable!(),
        });
    }

    None
}

pub(crate) fn task_state_needs_terminal_result(task_state: &TaskState) -> bool {
    match task_state.shape.kind {
        TaskKind::Output => !task_state.progress.output_verified,
        TaskKind::Answer | TaskKind::Transform => !task_state_has_terminal_result(task_state),
        TaskKind::Discovery | TaskKind::Unknown => false,
    }
}

pub(crate) fn required_input_is_satisfied(input: &RequiredInput) -> bool {
    matches!(input.status.as_str(), "ingested")
}

pub(crate) fn suggested_step_budget(default_max_steps: usize, shape: &TaskShape) -> usize {
    let minimum = match shape.kind {
        TaskKind::Output => 8,
        TaskKind::Transform => 6,
        TaskKind::Answer | TaskKind::Discovery | TaskKind::Unknown => default_max_steps,
    };
    let input_adjustment = shape.required_inputs.len().saturating_sub(1);
    default_max_steps.max(minimum + input_adjustment)
}

fn task_state_has_terminal_result(task_state: &TaskState) -> bool {
    if task_state.progress.output_verified {
        return true;
    }
    task_state
        .recent_actions
        .last()
        .map(|summary| action_summary_is_terminal(&summary.action))
        .unwrap_or(false)
}

fn state_has_terminal_result(state: &TaskLoopState, shape: &TaskShape) -> bool {
    shape
        .requested_output
        .as_ref()
        .map(|output| output.verified)
        .unwrap_or(false)
        || state
            .recent_action_summaries
            .last()
            .map(|summary| action_summary_is_terminal(&summary.action))
            .unwrap_or(false)
}

fn action_summary_is_terminal(action: &str) -> bool {
    action.starts_with("respond:")
        || action.starts_with("write_file:")
        || action.starts_with("append_file:")
        || action.starts_with("record_note:")
}

fn infer_task_kind(
    task_description: &str,
    requested_output: Option<&RequestedOutput>,
    required_inputs: &[RequiredInput],
) -> TaskKind {
    let lower = task_description.to_lowercase();
    let has_output_verb = contains_any(
        &lower,
        &["create", "write", "save", "generate", "produce", "make"],
    );
    let has_transform_verb = contains_any(
        &lower,
        &[
            "use",
            "combine",
            "fill",
            "fill out",
            "update",
            "modify",
            "edit",
            "rewrite",
            "transform",
            "convert",
            "merge",
            "using",
            "from",
        ],
    );
    let has_answer_verb = contains_any(
        &lower,
        &[
            "tell me",
            "what is",
            "what's",
            "which",
            "who",
            "where",
            "when",
            "summarize",
        ],
    );
    let has_discovery_verb = contains_any(
        &lower,
        &["find", "locate", "search", "look for", "list", "inspect"],
    );

    if requested_output.is_some() {
        TaskKind::Output
    } else if has_answer_verb && !has_output_verb {
        TaskKind::Answer
    } else if has_transform_verb && !required_inputs.is_empty() {
        TaskKind::Transform
    } else if has_output_verb {
        TaskKind::Output
    } else if has_discovery_verb {
        TaskKind::Discovery
    } else {
        TaskKind::Unknown
    }
}

fn build_success_markers(
    kind: TaskKind,
    required_inputs: &[RequiredInput],
    requested_output: Option<&RequestedOutput>,
) -> Vec<String> {
    let mut markers = Vec::new();

    if !required_inputs.is_empty() {
        markers.push("all required named inputs are ingested".to_string());
    }
    if let Some(output) = requested_output {
        markers.push(format!(
            "requested output {} is created and verified",
            output.locator_hint
        ));
    }
    if matches!(kind, TaskKind::Answer) {
        markers.push("final response is grounded in gathered evidence".to_string());
    }
    if markers.is_empty() {
        markers.push("task-specific requested work is complete".to_string());
    }

    markers
}

fn file_mentions_with_positions(task_description: &str) -> Vec<(usize, String)> {
    task_description
        .split_whitespace()
        .enumerate()
        .filter_map(|(index, raw)| {
            let cleaned = clean_locator_hint(raw);
            if !is_file_like_hint(&cleaned) {
                return None;
            }
            Some((index, cleaned))
        })
        .collect()
}

fn infer_requested_output(
    task_description: &str,
    file_mentions: &[(usize, String)],
    state: &TaskLoopState,
) -> Option<RequestedOutput> {
    let tokens = task_description
        .split_whitespace()
        .map(clean_locator_hint)
        .collect::<Vec<_>>();

    file_mentions.iter().rev().find_map(|(index, hint)| {
        let start = index.saturating_sub(4);
        let cue_window = tokens
            .get(start..=*index)
            .unwrap_or(&[])
            .iter()
            .map(|token| token.to_lowercase())
            .collect::<Vec<_>>();

        if !cue_window.iter().any(|token| {
            matches!(
                token.as_str(),
                "save"
                    | "as"
                    | "to"
                    | "into"
                    | "create"
                    | "write"
                    | "update"
                    | "modify"
                    | "edit"
                    | "rewrite"
                    | "append"
                    | "overwrite"
                    | "fill"
                    | "named"
                    | "called"
                    | "output"
                    | "file"
            )
        }) {
            return None;
        }

        let (exists, verified) = infer_requested_output_status(hint, state);
        Some(RequestedOutput {
            locator_hint: hint.clone(),
            kind: classify_locator_kind(hint),
            exists,
            verified,
        })
    })
}

fn infer_required_input_status(hint: &str, state: &TaskLoopState) -> String {
    let mut best_priority = 0;
    let mut best_status = "unresolved";

    for source in &state.working_sources {
        if !locator_matches_hint(&source.locator, hint) {
            continue;
        }
        let (status, priority) = match source.status.as_str() {
            "read" | "excerpted" | "matched_text" => ("ingested", 2),
            "matched" | "listed" | "inspected" => ("located", 1),
            _ if source.role == "authoritative" => ("ingested", 2),
            _ => ("located", 1),
        };
        if priority > best_priority {
            best_priority = priority;
            best_status = status;
        }
    }

    for artifact in &state.artifact_references {
        if !locator_matches_hint(&artifact.locator, hint) {
            continue;
        }
        let (status, priority) = match artifact.status.as_str() {
            "read" | "extracted" | "searched" => ("ingested", 2),
            "matched" | "listed" | "inspected" => ("located", 1),
            _ => continue,
        };
        if priority > best_priority {
            best_priority = priority;
            best_status = status;
        }
    }

    best_status.to_string()
}

fn infer_requested_output_status(hint: &str, state: &TaskLoopState) -> (bool, bool) {
    let mut exists = false;
    let mut verified = false;

    for source in &state.working_sources {
        if !locator_matches_hint(&source.locator, hint) {
            continue;
        }
        exists = true;
        if source.role == "generated" || matches!(source.status.as_str(), "written" | "appended") {
            verified = true;
        }
    }

    for artifact in &state.artifact_references {
        if !locator_matches_hint(&artifact.locator, hint) {
            continue;
        }
        exists = true;
        if matches!(artifact.status.as_str(), "written" | "appended") {
            verified = true;
        }
    }

    (exists, verified)
}

fn classify_locator_kind(locator: &str) -> String {
    Path::new(locator)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "file".to_string())
}

fn clean_locator_hint(raw: &str) -> String {
    raw.trim_matches(|character: char| {
        !(character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | '-' | '/' | '~' | '\\'))
    })
    .to_string()
}

fn is_file_like_hint(hint: &str) -> bool {
    let Some(extension) = Path::new(hint).extension().and_then(|value| value.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "txt"
            | "md"
            | "pdf"
            | "csv"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "rs"
            | "js"
            | "ts"
            | "tsx"
            | "py"
            | "sh"
            | "sql"
    )
}

fn locator_matches_hint(locator: &str, hint: &str) -> bool {
    let normalized_locator = normalize_locator(locator);
    let normalized_hint = normalize_locator(hint);
    normalized_locator.contains(&normalized_hint) || normalized_hint.contains(&normalized_locator)
}

fn normalize_locator(locator: &str) -> String {
    locator
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}
