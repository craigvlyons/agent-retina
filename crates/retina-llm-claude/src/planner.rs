use retina_types::*;
use serde_json;
use std::path::PathBuf;

pub fn plan_task(task: &str, last_result: Option<&str>) -> Option<ReasonResponse> {
    let trimmed = task.trim();
    if trimmed.is_empty() {
        return Some(respond(
            "I need a task to act on. Try asking me to list files, read a file, search text, write a file, or run a command.",
        ));
    }

    if is_greeting(trimmed) || is_capability_question(trimmed) {
        return Some(respond(&capability_message()));
    }

    if let Some(action) = plan_follow_up_action(trimmed, last_result) {
        return Some(with_reasoning(action, true, "deterministic planner: follow-up action"));
    }

    if let Some(action) = plan_write_action(trimmed) {
        return Some(with_reasoning(action, true, "deterministic planner: write action"));
    }

    if let Some(action) = plan_read_action(trimmed) {
        return Some(with_reasoning(action, true, "deterministic planner: read action"));
    }

    if let Some(action) = plan_search_action(trimmed) {
        return Some(with_reasoning(
            action,
            !task_requires_search_follow_up(trimmed),
            "deterministic planner: search action",
        ));
    }

    if let Some(action) = plan_find_action(trimmed) {
        return Some(with_reasoning(
            action,
            !task_requires_find_follow_up(trimmed),
            "deterministic planner: find action",
        ));
    }

    if let Some(action) = plan_list_action(trimmed) {
        return Some(with_reasoning(action, true, "deterministic planner: list action"));
    }

    if let Some(action) = plan_inspect_action(trimmed) {
        return Some(with_reasoning(action, true, "deterministic planner: inspect action"));
    }

    if let Some(command) = extract_command(trimmed) {
        return Some(with_reasoning(
            command_action(command),
            true,
            "deterministic planner: explicit command",
        ));
    }

    None
}

pub fn reflect_task(task: &str, last_result: Option<&str>) -> ReasonResponse {
    if let Some(path) = extract_path(task) {
        return with_reasoning(
            Action::ReadFile {
                id: ActionId::new(),
                path,
                max_bytes: Some(16 * 1024),
            },
            true,
            "reflection fallback: inspect the relevant file",
        );
    }

    let detail = last_result.unwrap_or("The previous action failed or produced no useful delta.");
    respond(&format!(
        "{detail} I should switch to a narrower file inspection, text search, or a safer write action next."
    ))
}

pub fn fallback_response(task: &str) -> ReasonResponse {
    if let Some(response) = plan_task(task, None) {
        return response;
    }

    respond(
        "I can act on concrete CLI tasks: list directories, find files, read files, search text, write or append files with approval, and run controlled shell commands. Give me a specific task.",
    )
}

pub fn capability_message() -> String {
    "I can list directories, find files, read files, search text across the repo, write or append files with approval, inspect paths, record notes, and run controlled shell commands. Give me a concrete task and I will route it through the kernel.".to_string()
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
    [
        "what can you do",
        "do you only",
        "can you do",
        "help",
        "how do i use",
        "what do you do",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn plan_list_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("list")
        || lower.contains("show directory")
        || lower == "ls"
        || lower.contains("working directory")
    {
        return Some(Action::ListDirectory {
            id: ActionId::new(),
            path: extract_path(task).unwrap_or_else(|| PathBuf::from(".")),
            recursive: lower.contains("recursive") || lower.contains("recursively"),
            max_entries: 100,
        });
    }
    None
}

fn plan_find_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("find file") || lower.contains("find files") || lower.contains("locate ") {
        let pattern = extract_quoted(task)
            .or_else(|| {
                extract_after_keyword(
                    task,
                    &[
                        "named",
                        "called",
                        "matching",
                        "find file",
                        "find files",
                        "locate",
                    ],
                )
            })
            .unwrap_or_else(|| task.to_string());
        return Some(Action::FindFiles {
            id: ActionId::new(),
            root: PathBuf::from("."),
            pattern: clean_fragment(&pattern),
            max_results: 50,
        });
    }
    None
}

fn plan_search_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("search") || lower.contains("grep") || lower.contains("look for") {
        let query = extract_quoted(task)
            .or_else(|| extract_after_keyword(task, &["for", "search", "grep", "look for"]))
            .unwrap_or_else(|| task.to_string());
        return Some(Action::SearchText {
            id: ActionId::new(),
            root: PathBuf::from("."),
            query: clean_fragment(&query),
            max_results: 25,
        });
    }
    None
}

fn plan_read_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("read ")
        || lower.contains("open ")
        || lower.contains("show file")
        || lower.contains("cat ")
    {
        let path = extract_path(task)?;
        return Some(Action::ReadFile {
            id: ActionId::new(),
            path,
            max_bytes: Some(24 * 1024),
        });
    }
    None
}

fn plan_write_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("append ") {
        let content = extract_quoted(task)?;
        let path =
            extract_path_after_keyword(task, &[" to ", " into "]).or_else(|| extract_path(task))?;
        return Some(Action::AppendFile {
            id: ActionId::new(),
            path,
            content,
            require_approval: true,
        });
    }

    if lower.contains("write ") || lower.contains("create file") {
        let content = extract_quoted(task)?;
        let path = extract_path_after_keyword(task, &[" to ", " into ", " at "])
            .or_else(|| extract_path(task))?;
        return Some(Action::WriteFile {
            id: ActionId::new(),
            path,
            content,
            overwrite: lower.contains("overwrite") || lower.contains("replace"),
            require_approval: true,
        });
    }

    None
}

fn plan_inspect_action(task: &str) -> Option<Action> {
    let lower = task.to_lowercase();
    if lower.contains("inspect ") || lower.contains("stat ") || lower.contains("metadata ") {
        let path = extract_path(task)?;
        return Some(Action::InspectPath {
            id: ActionId::new(),
            path,
            include_content: lower.contains("content"),
        });
    }
    None
}

fn plan_follow_up_action(task: &str, last_result: Option<&str>) -> Option<Action> {
    let last_result = last_result?;
    let result: ActionResult = serde_json::from_str(last_result).ok()?;
    let lower = task.to_lowercase();

    match result {
        ActionResult::FileMatches { matches, .. } if task_requires_find_follow_up(task) => {
            let path = pick_preferred_path(&matches)?;
            if lower.contains("read") || lower.contains("open") {
                Some(Action::ReadFile {
                    id: ActionId::new(),
                    path,
                    max_bytes: Some(24 * 1024),
                })
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
            Some(Action::ReadFile {
                id: ActionId::new(),
                path,
                max_bytes: Some(24 * 1024),
            })
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
            || lower.contains(" then inspect"))
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

pub fn extract_command(task: &str) -> Option<String> {
    let trimmed = task.trim();
    trimmed
        .strip_prefix("run ")
        .or_else(|| trimmed.strip_prefix("execute "))
        .map(|command| command.trim().to_string())
        .or_else(|| extract_backticked(trimmed))
}

fn command_action(command: String) -> Action {
    Action::RunCommand {
        id: ActionId::new(),
        expect_change: command_expects_change(&command),
        require_approval: command_requires_approval(&command),
        state_scope: HashScope {
            tracked_paths: Vec::new(),
            include_working_directory: true,
            include_last_command: true,
        },
        cwd: None,
        command,
    }
}

fn command_expects_change(command: &str) -> bool {
    let lower = command.to_lowercase();
    if lower.contains('>') || lower.contains(">>") {
        return true;
    }
    ![
        "ls",
        "pwd",
        "cat ",
        "rg ",
        "grep ",
        "find ",
        "git status",
        "git diff",
        "wc ",
        "head ",
        "tail ",
        "stat ",
    ]
    .iter()
    .any(|marker| lower == *marker || lower.starts_with(marker))
}

fn command_requires_approval(command: &str) -> bool {
    let lower = command.to_lowercase();
    [
        "rm ",
        "mv ",
        "chmod ",
        "chown ",
        "truncate ",
        "sed -i",
        "perl -pi",
        ">",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn extract_quoted(input: &str) -> Option<String> {
    input
        .split('"')
        .nth(1)
        .map(|value| value.trim().to_string())
        .or_else(|| {
            input
                .split('\'')
                .nth(1)
                .map(|value| value.trim().to_string())
        })
}

fn extract_backticked(input: &str) -> Option<String> {
    input
        .split('`')
        .nth(1)
        .map(|value| value.trim().to_string())
}

fn extract_after_keyword(input: &str, keywords: &[&str]) -> Option<String> {
    let lower = input.to_lowercase();
    for keyword in keywords {
        if let Some(index) = lower.find(keyword) {
            let value = input[index + keyword.len()..].trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn extract_path_after_keyword(input: &str, keywords: &[&str]) -> Option<PathBuf> {
    let lower = input.to_lowercase();
    for keyword in keywords {
        if let Some(index) = lower.find(keyword) {
            let remainder = &input[index + keyword.len()..];
            if let Some(path) = parse_path_token(remainder) {
                return Some(path);
            }
        }
    }
    None
}

fn extract_path(input: &str) -> Option<PathBuf> {
    extract_quoted_path(input).or_else(|| parse_path_token(input))
}

fn extract_quoted_path(input: &str) -> Option<PathBuf> {
    for quote in ['"', '\''] {
        let mut parts = input.split(quote);
        let _ = parts.next();
        if let Some(candidate) = parts.next() {
            let candidate = candidate.trim();
            if looks_like_path(candidate) {
                return Some(PathBuf::from(candidate));
            }
        }
    }
    None
}

fn parse_path_token(input: &str) -> Option<PathBuf> {
    input
        .split_whitespace()
        .map(clean_token)
        .find(|token| looks_like_path(token))
        .map(PathBuf::from)
}

fn looks_like_path(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.contains('/')
        || token.contains('.')
        || token == "."
}

fn clean_fragment(value: &str) -> String {
    value
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '.' || c == '?' || c == '!')
        .to_string()
}

fn clean_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        c == '"'
            || c == '\''
            || c == ','
            || c == '.'
            || c == ':'
            || c == ';'
            || c == ')'
            || c == '('
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_read_file_task() {
        let response = plan_task("read README.md", None).unwrap();
        assert!(matches!(response.action, Action::ReadFile { .. }));
        assert!(response.task_complete);
    }

    #[test]
    fn plans_capability_response() {
        let response = plan_task("can you do anything but cwd commands", None).unwrap();
        let Action::Respond { message, .. } = response.action else {
            panic!("expected response action");
        };
        assert!(message.contains("list directories"));
    }

    #[test]
    fn plans_write_file_task() {
        let response = plan_task("write \"hello\" to notes/todo.txt", None).unwrap();
        let Action::WriteFile { path, content, .. } = response.action else {
            panic!("expected write file action");
        };
        assert_eq!(path, PathBuf::from("notes/todo.txt"));
        assert_eq!(content, "hello");
    }

    #[test]
    fn plans_follow_up_read_after_find() {
        let previous = serde_json::to_string(&ActionResult::FileMatches {
            root: ".".into(),
            pattern: "Cargo.toml".to_string(),
            matches: vec!["Cargo.toml".into(), "crates/retina-cli/Cargo.toml".into()],
        })
        .unwrap();
        let response =
            plan_task("find files named Cargo.toml and read the root one", Some(&previous)).unwrap();
        assert!(matches!(response.action, Action::ReadFile { .. }));
        assert!(response.task_complete);
    }
}
