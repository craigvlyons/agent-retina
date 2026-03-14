use retina_types::*;
use std::path::PathBuf;

pub fn plan_task(task: &str, last_result: Option<&str>) -> Option<ReasonResponse> {
    let trimmed = task.trim();
    if trimmed.is_empty() {
        return Some(respond(
            "I need a task to act on. Try asking me to inspect, read, search, or modify something concrete.",
        ));
    }

    if is_greeting(trimmed) || is_capability_question(trimmed) {
        return Some(respond(&capability_message()));
    }

    if let Some(action) = plan_follow_up_action(trimmed, last_result) {
        return Some(with_reasoning(
            action,
            true,
            "deterministic planner: structured follow-up from prior result",
        ));
    }

    None
}

pub fn capability_message() -> String {
    "I can explore and act through the CLI shell: inspect paths, list directories, find files, search text, read files, extract text from documents, write or append files with approval, record notes, and run controlled shell commands. For concrete work, I should reason about the task and choose the next action through the kernel.".to_string()
}

fn with_reasoning(action: Action, task_complete: bool, reasoning: &str) -> ReasonResponse {
    ReasonResponse {
        action,
        task_complete,
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

fn plan_follow_up_action(task: &str, last_result: Option<&str>) -> Option<Action> {
    let last_result = last_result?;
    let result: ActionResult = serde_json::from_str(last_result).ok()?;
    let lower = task.to_lowercase();

    match result {
        ActionResult::FileMatches { matches, .. } if task_requires_find_follow_up(task) => {
            let path = pick_preferred_path(&matches)?;
            if mentions_directory_contents(task) {
                Some(Action::ListDirectory {
                    id: ActionId::new(),
                    path,
                    recursive: false,
                    max_entries: 100,
                })
            } else if lower.contains("read") || lower.contains("open") {
                Some(content_action_for_path(path))
            } else if lower.contains("inspect") {
                Some(Action::InspectPath {
                    id: ActionId::new(),
                    path,
                    include_content: true,
                })
            } else {
                None
            }
        }
        ActionResult::TextSearch { matches, .. } if task_requires_search_follow_up(task) => {
            let path = pick_preferred_path(
                &matches
                    .into_iter()
                    .map(|item| item.path)
                    .collect::<Vec<_>>(),
            )?;
            Some(content_action_for_path(path))
        }
        _ => None,
    }
}

fn task_requires_find_follow_up(task: &str) -> bool {
    let lower = task.to_lowercase();
    (lower.contains("find") || lower.contains("locate"))
        && (lower.contains(" and read")
            || lower.contains(" then read")
            || lower.contains(" and open")
            || lower.contains(" then open")
            || lower.contains(" and inspect")
            || lower.contains(" then inspect")
            || mentions_directory_contents(&lower))
}

fn task_requires_search_follow_up(task: &str) -> bool {
    let lower = task.to_lowercase();
    (lower.contains("search") || lower.contains("grep") || lower.contains("look for"))
        && (lower.contains(" and read")
            || lower.contains(" then read")
            || lower.contains(" and open")
            || lower.contains(" then open"))
}

fn pick_preferred_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .min_by_key(|path| path.components().count())
        .cloned()
}

fn mentions_directory_contents(task: &str) -> bool {
    let lower = task.to_lowercase();
    lower.contains("what files are there")
        || lower.contains("tell me what files")
        || lower.contains("show me the files")
        || lower.contains("list the files")
        || lower.contains("what is in")
        || lower.contains("what's in")
}

fn content_action_for_path(path: PathBuf) -> Action {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
    {
        Action::ExtractDocumentText {
            id: ActionId::new(),
            path,
            max_chars: Some(24 * 1024),
        }
    } else {
        Action::ReadFile {
            id: ActionId::new(),
            path,
            max_bytes: Some(24 * 1024),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_capability_response() {
        let response = plan_task("what can you do", None).unwrap();
        let Action::Respond { message, .. } = response.action else {
            panic!("expected response action");
        };
        assert!(message.contains("CLI shell"));
    }

    #[test]
    fn plans_follow_up_read_after_find() {
        let previous = serde_json::to_string(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Cargo.toml".to_string(),
            matches: vec!["Cargo.toml".into(), "crates/retina-cli/Cargo.toml".into()],
        })
        .unwrap();
        let response = plan_task(
            "find files named Cargo.toml and read the root one",
            Some(&previous),
        )
        .unwrap();
        assert!(matches!(response.action, Action::ReadFile { .. }));
        assert!(response.task_complete);
    }

    #[test]
    fn plans_follow_up_document_extract_after_pdf_find() {
        let previous = serde_json::to_string(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Craig Lyons.pdf".to_string(),
            matches: vec!["Craig Lyons.pdf".into()],
        })
        .unwrap();
        let response = plan_task("find the resume and read it", Some(&previous)).unwrap();
        assert!(matches!(response.action, Action::ExtractDocumentText { .. }));
    }

    #[test]
    fn plans_follow_up_directory_listing_after_find() {
        let previous = serde_json::to_string(&ActionResult::FileMatches {
            root: "Desktop".into(),
            pattern: "resume".to_string(),
            matches: vec!["Desktop/resume".into()],
        })
        .unwrap();
        let response = plan_task(
            "find the resume folder and tell me what files are there",
            Some(&previous),
        )
        .unwrap();
        assert!(matches!(response.action, Action::ListDirectory { .. }));
    }

    #[test]
    fn concrete_task_without_prior_result_is_not_short_circuited() {
        assert!(plan_task("find the resume file on desktop and tell me what is in it", None).is_none());
    }
}
