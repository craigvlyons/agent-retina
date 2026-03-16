use retina_types::*;
use std::path::{Path, PathBuf};

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

fn plan_follow_up_action(task: &str, last_result: Option<&str>) -> Option<Action> {
    let last_result = last_result?;
    let result: ActionResult = serde_json::from_str(last_result).ok()?;
    let lower = task.to_lowercase();

    match result {
        ActionResult::FileMatches { matches, .. } if task_requires_content_follow_up(task) => {
            let path = pick_preferred_path(task, &matches)?;
            if mentions_directory_contents(task) && matched_path_looks_like_directory(&path) {
                Some(Action::ListDirectory {
                    id: ActionId::new(),
                    path,
                    recursive: false,
                    max_entries: 100,
                })
            } else if lower.contains("inspect") {
                Some(Action::InspectPath {
                    id: ActionId::new(),
                    path,
                    include_content: true,
                })
            } else if asks_for_content_answer(task)
                || task_mentions_document_page(task)
                || lower.contains("read")
                || lower.contains("open")
                || lower.contains("summarize")
            {
                Some(content_action_for_path(task, path))
            } else {
                None
            }
        }
        ActionResult::TextSearch { matches, .. } if task_requires_search_follow_up(task) => {
            let path = pick_preferred_path(
                task,
                &matches
                    .into_iter()
                    .map(|item| item.path)
                    .collect::<Vec<_>>(),
            )?;
            Some(content_action_for_path(task, path))
        }
        _ => None,
    }
}

fn task_requires_content_follow_up(task: &str) -> bool {
    let lower = task.to_lowercase();
    (lower.contains("find") || lower.contains("locate"))
        && (lower.contains(" and read")
            || lower.contains(" then read")
            || lower.contains(" and open")
            || lower.contains(" then open")
            || lower.contains(" and inspect")
            || lower.contains(" then inspect")
            || asks_for_content_answer(task)
            || lower.contains("summarize")
            || task_mentions_document_page(task)
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

fn pick_preferred_path(task: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .min_by_key(|path| {
            (
                task_path_rank(task, path),
                path.components().count(),
                path.to_string_lossy().len(),
            )
        })
        .cloned()
}

fn task_path_rank(task: &str, path: &Path) -> usize {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    let wants_structured = task_mentions_structured_data(task);
    let wants_document_pages = task_mentions_document_page(task);

    if path.is_dir() {
        return if mentions_directory_contents(task) {
            0
        } else {
            3
        };
    }

    if wants_structured && matches!(extension.as_deref(), Some("csv" | "tsv")) {
        return 0;
    }
    if wants_structured {
        return 2;
    }

    if wants_document_pages && matches!(extension.as_deref(), Some("pdf")) {
        return 0;
    }
    if wants_document_pages {
        return 2;
    }

    readability_rank(path)
}

fn readability_rank(path: &Path) -> usize {
    if path.is_dir() {
        return 0;
    }
    match path.extension().and_then(|value| value.to_str()) {
        Some(ext)
            if matches!(
                ext.to_ascii_lowercase().as_str(),
                "md" | "txt" | "rs" | "toml" | "json" | "yaml" | "yml" | "js" | "ts" | "tsx"
            ) =>
        {
            0
        }
        Some("pdf") => 1,
        Some(_) => 2,
        None => 1,
    }
}

fn task_mentions_structured_data(task: &str) -> bool {
    let lower = task.to_lowercase();
    [
        "csv",
        "tsv",
        "table",
        "rows",
        "columns",
        "headers",
        "spreadsheet",
        "data",
        "records",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn task_mentions_document_page(task: &str) -> bool {
    let lower = task.to_lowercase();
    lower.contains("page ") || lower.contains("pages ")
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

fn matched_path_looks_like_directory(path: &Path) -> bool {
    path.is_dir() || path.extension().is_none()
}

fn asks_for_content_answer(task: &str) -> bool {
    let lower = task.to_lowercase();
    [
        "tell me",
        "summarize",
        "what is",
        "what's",
        "which",
        "who",
        "where",
        "when",
        "last job",
        "latest role",
        "most recent",
        "current position",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn content_action_for_path(task: &str, path: PathBuf) -> Action {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("csv" | "tsv")) {
        Action::IngestStructuredData {
            id: ActionId::new(),
            path,
            max_rows: Some(25),
        }
    } else if matches!(extension.as_deref(), Some("pdf")) {
        let (page_start, page_end) = requested_page_range(task);
        Action::ExtractDocumentText {
            id: ActionId::new(),
            path,
            max_chars: Some(24 * 1024),
            page_start,
            page_end,
        }
    } else {
        Action::ReadFile {
            id: ActionId::new(),
            path,
            max_bytes: Some(24 * 1024),
        }
    }
}

fn requested_page_range(task: &str) -> (Option<usize>, Option<usize>) {
    let tokens = task
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();

    for window in tokens.windows(4) {
        if matches!(window[0].as_str(), "pages" | "page")
            && matches!(window[2].as_str(), "to" | "through" | "thru")
        {
            if let (Ok(start), Ok(end)) = (window[1].parse::<usize>(), window[3].parse::<usize>()) {
                return (Some(start), Some(end));
            }
        }
    }

    for window in tokens.windows(2) {
        if matches!(window[0].as_str(), "page" | "pages") {
            if let Ok(page) = window[1].parse::<usize>() {
                return (Some(page), Some(page));
            }
        }
    }

    (None, None)
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
    fn plans_follow_up_read_after_find() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Cargo.toml".to_string(),
            matches: vec!["Cargo.toml".into(), "crates/retina-cli/Cargo.toml".into()],
        });
        let response = plan_task(
            "find files named Cargo.toml and read the root one",
            Some(&previous),
        );
        let response = must(response, "expected follow-up read plan");
        assert!(matches!(response.action, Action::ReadFile { .. }));
        assert!(response.task_complete);
    }

    #[test]
    fn plans_follow_up_document_extract_after_pdf_find() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Craig Lyons.pdf".to_string(),
            matches: vec!["Craig Lyons.pdf".into()],
        });
        let response = must(
            plan_task("find the resume and read it", Some(&previous)),
            "expected document extract plan",
        );
        assert!(matches!(
            response.action,
            Action::ExtractDocumentText { .. }
        ));
    }

    #[test]
    fn plans_follow_up_structured_ingest_after_csv_find() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "people.csv".to_string(),
            matches: vec!["people.csv".into()],
        });
        let response = must(
            plan_task("find the csv and tell me what is in it", Some(&previous)),
            "expected structured ingest plan",
        );
        assert!(matches!(
            response.action,
            Action::IngestStructuredData { .. }
        ));
    }

    #[test]
    fn prefers_structured_candidate_for_data_question() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "people".to_string(),
            matches: vec!["people.md".into(), "people.csv".into()],
        });
        let response = must(
            plan_task(
                "find the people data and tell me what rows are in it",
                Some(&previous),
            ),
            "expected structured candidate preference",
        );
        match response.action {
            Action::IngestStructuredData { path, .. } => assert!(path.ends_with("people.csv")),
            _ => panic!("expected structured ingest"),
        }
    }

    #[test]
    fn prefers_pdf_candidate_for_page_specific_question() {
        let previous = to_json(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "dominican".to_string(),
            matches: vec!["dominican.txt".into(), "dominican_Med.pdf".into()],
        });
        let response = must(
            plan_task("find dominican and use page 2 from it", Some(&previous)),
            "expected pdf candidate preference",
        );
        match response.action {
            Action::ExtractDocumentText {
                path,
                page_start,
                page_end,
                ..
            } => {
                assert!(path.ends_with("dominican_Med.pdf"));
                assert_eq!(page_start, Some(2));
                assert_eq!(page_end, Some(2));
            }
            _ => panic!("expected document extraction"),
        }
    }

    #[test]
    fn plans_follow_up_directory_listing_after_find() {
        let previous = to_json(&ActionResult::FileMatches {
            root: "Desktop".into(),
            pattern: "resume".to_string(),
            matches: vec!["Desktop/resume".into()],
        });
        let response = plan_task(
            "find the resume folder and tell me what files are there",
            Some(&previous),
        );
        let response = must(response, "expected directory listing follow-up");
        assert!(matches!(response.action, Action::ListDirectory { .. }));
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
    fn plans_follow_up_read_for_content_question_after_find() {
        let previous = to_json(&ActionResult::FileMatches {
            root: "Desktop/resume".into(),
            pattern: "Craig Lyons resume.md".to_string(),
            matches: vec!["Desktop/resume/Craig Lyons resume.md".into()],
        });
        let response = plan_task(
            "find the craig lyons resume.md file and tell me what my last job was",
            Some(&previous),
        );
        let response = must(response, "expected read follow-up for content question");
        assert!(matches!(response.action, Action::ReadFile { .. }));
        assert!(response.task_complete);
    }
}

