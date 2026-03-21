use retina_types::*;
pub fn plan_task(task: &str, last_result: Option<&str>) -> Option<ReasonResponse> {
    let trimmed = task.trim();
    let _ = last_result;
    if trimmed.is_empty() {
        return Some(respond(
            "I need a task to act on. Try asking me to inspect, read, search, or modify something concrete.",
        ));
    }

    if is_greeting(trimmed) || is_capability_question(trimmed) {
        return Some(respond(&capability_message()));
    }

    None
}

pub fn capability_message() -> String {
    "I can explore and act through the CLI shell: inspect paths, list directories, find files, search text, read files, ingest structured local data, extract text from documents, create or modify files, record notes, and run shell commands when they help complete the task. For concrete work, I should reason about the task and choose the next action through the kernel.".to_string()
}

fn with_reasoning(action: Action, task_complete: bool, reasoning: &str) -> ReasonResponse {
    ReasonResponse {
        action,
        task_complete,
        framing: None,
        reasoning: Some(reasoning.to_string()),
        tokens_used: TokenUsage::default(),
    }
}

fn respond(message: &str) -> ReasonResponse {
    with_reasoning(
        Action::Respond {
            id: ActionId::new(),
            message: message.to_string(),
        },
        true,
        "deterministic planner: operator response",
    )
}

fn is_greeting(task: &str) -> bool {
    matches!(task.to_lowercase().as_str(), "hi" | "hello" | "hey" | "yo")
}

fn is_capability_question(task: &str) -> bool {
    let lower = task.to_lowercase();
    let trimmed = lower.trim();
    if matches!(
        trimmed,
        "help" | "/help" | "what can you do" | "what do you do"
    ) {
        return true;
    }

    [
        "what can you do",
        "what do you do",
        "what are your capabilities",
        "what can retina do",
        "how do i use",
        "do you only",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must<T>(value: Option<T>, message: &str) -> T {
        value.unwrap_or_else(|| panic!("{message}"))
    }

    fn to_json(value: &ActionResult) -> String {
        serde_json::to_string(value)
            .unwrap_or_else(|error| panic!("failed to serialize test action result: {error}"))
    }

    #[test]
    fn plans_capability_response() {
        let response = must(
            plan_task("what can you do", None),
            "expected planner response",
        );
        let Action::Respond { message, .. } = response.action else {
            panic!("expected response action");
        };
        assert!(message.contains("CLI shell"));
    }

    #[test]
    fn concrete_task_without_prior_result_is_not_short_circuited() {
        assert!(
            plan_task(
                "find the resume file on desktop and tell me what is in it",
                None
            )
            .is_none()
        );
    }

    #[test]
    fn prior_results_do_not_trigger_deterministic_follow_up_plans() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Cargo.toml".to_string(),
            matches: vec!["Cargo.toml".into()],
        });
        assert!(plan_task("find files named Cargo.toml and read the root one", Some(&previous)).is_none());
    }
}
