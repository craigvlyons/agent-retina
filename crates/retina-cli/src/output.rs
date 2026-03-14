use retina_memory_sqlite::MemoryStats;
use retina_types::*;
use serde_json::Value;

pub fn render_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Command(result) => format!(
            "Command completed (exit={:?})\nstdout:\n{}\nstderr:\n{}",
            result.exit_code, result.stdout, result.stderr
        ),
        ActionResult::Inspection(state) => {
            format!("Inspection complete for cwd {}", state.cwd.display())
        }
        ActionResult::DirectoryListing { root, entries } => format!(
            "Directory listing for {}\n{}",
            root.display(),
            entries
                .iter()
                .map(|entry| {
                    let kind = if entry.is_dir { "dir" } else { "file" };
                    format!("- [{}] {}", kind, entry.path.display())
                })
                .collect::<Vec<_>>()
                .join("\n")
        ),
        ActionResult::FileMatches {
            root,
            pattern,
            matches,
        } => format!(
            "Paths matching '{pattern}' under {}\n{}",
            root.display(),
            if matches.is_empty() {
                "(no matches)".to_string()
            } else {
                matches
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        ActionResult::FileRead {
            path,
            content,
            truncated,
        } => format!(
            "Read file {}\n{}\n{}",
            path.display(),
            content,
            if *truncated {
                "\n[output truncated]"
            } else {
                ""
            }
        ),
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            format,
        } => format!(
            "Extracted {format} text from {}\n{}\n{}",
            path.display(),
            content,
            if *truncated {
                "\n[output truncated]"
            } else {
                ""
            }
        ),
        ActionResult::TextSearch {
            root,
            query,
            matches,
        } => format!(
            "Text search for '{query}' under {}\n{}",
            root.display(),
            if matches.is_empty() {
                "(no matches)".to_string()
            } else {
                matches
                    .iter()
                    .map(|item| {
                        format!(
                            "- {}:{} {}",
                            item.path.display(),
                            item.line_number,
                            item.line
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        ActionResult::FileWrite {
            path,
            bytes_written,
            appended,
        } => format!(
            "{} {} byte(s) {}",
            if *appended { "Appended" } else { "Wrote" },
            bytes_written,
            path.display()
        ),
        ActionResult::NoteRecorded { note } => format!("Recorded note: {note}"),
        ActionResult::Response { message } => message.clone(),
    }
}

pub fn render_timeline(events: &[TimelineEvent]) -> String {
    let mut output = String::new();
    for event in events {
        output.push_str(&render_timeline_event(event));
    }
    output
}

pub fn render_timeline_event(event: &TimelineEvent) -> String {
    format!(
        "[{}] {:?} task={}{}\n",
        event.timestamp.to_rfc3339(),
        event.event_type,
        event.task_id,
        event
            .delta_summary
            .as_ref()
            .map(|summary| format!(" delta={summary}"))
            .unwrap_or_default()
    )
}

pub fn render_chat_event(event: &TimelineEvent, debug: bool) -> String {
    if debug {
        return render_timeline_event(event);
    }

    let line = match event.event_type {
        TimelineEventType::TaskReceived => {
            event.payload_json.get("task").and_then(Value::as_str).map(|task| {
                format!("task: {task}")
            })
        }
        TimelineEventType::ReasonerCalled => {
            if let Some(action) = event.payload_json.get("action").and_then(Value::as_str) {
                Some(format!("plan: {}", humanize_action_label(action)))
            } else {
                event.payload_json
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .map(|reasoning| format!("thinking: {reasoning}"))
            }
        }
        TimelineEventType::ReflexSelected => event
            .payload_json
            .get("action")
            .and_then(Value::as_str)
            .map(|action| format!("reflex: {}", humanize_action_label(action))),
        TimelineEventType::ActionDispatched => event
            .payload_json
            .get("action")
            .and_then(Value::as_str)
            .map(|action| format!("action: {}", humanize_action_label(action))),
        TimelineEventType::ReflectionRequested => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("reflection: {reason}")),
        TimelineEventType::TaskFailed => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("failed: {reason}")),
        TimelineEventType::TaskCancelled => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("cancelled: {reason}")),
        TimelineEventType::TaskCompleted => Some("done".to_string()),
        _ => None,
    };

    line.map(|line| format!("{line}\n")).unwrap_or_default()
}

fn humanize_action_label(label: &str) -> String {
    if let Some(value) = label.strip_prefix("run_command:") {
        return format!("run command `{value}`");
    }
    if let Some(value) = label.strip_prefix("read_file:") {
        return format!("read file {}", value);
    }
    if let Some(value) = label.strip_prefix("extract_document_text:") {
        return format!("extract text from {}", value);
    }
    if let Some(value) = label.strip_prefix("write_file:") {
        return format!("write file {}", value);
    }
    if let Some(value) = label.strip_prefix("append_file:") {
        return format!("append file {}", value);
    }
    if let Some(value) = label.strip_prefix("inspect_path:") {
        return format!("inspect path {}", value);
    }
    if let Some(value) = label.strip_prefix("record_note:") {
        return format!("record note {}", value);
    }
    if let Some(value) = label.strip_prefix("respond:") {
        return format!("respond {}", value);
    }
    if let Some(value) = label.strip_prefix("find_files:") {
        let mut parts = value.splitn(2, ':');
        let root = parts.next().unwrap_or(".");
        let pattern = parts.next().unwrap_or("*");
        return format!("find `{pattern}` under {root}");
    }
    if let Some(value) = label.strip_prefix("search_text:") {
        let mut parts = value.splitn(2, ':');
        let root = parts.next().unwrap_or(".");
        let query = parts.next().unwrap_or_default();
        return format!("search `{query}` under {root}");
    }
    if let Some(value) = label.strip_prefix("list_directory:") {
        let mut parts = value.splitn(2, ":recursive=");
        let path = parts.next().unwrap_or(".");
        let recursive = parts.next().unwrap_or("false");
        return if recursive == "true" {
            format!("list directory {} recursively", path)
        } else {
            format!("list directory {}", path)
        };
    }
    label.replace('_', " ")
}

pub fn render_memory_inspection(knowledge: &[KnowledgeNode], experiences: &[Experience]) -> String {
    let mut output = String::from("Knowledge:\n");
    for item in knowledge {
        output.push_str(&format!("- [{}] {}\n", item.category, item.content));
    }
    output.push_str("\nExperiences:\n");
    for item in experiences {
        output.push_str(&format!(
            "- {} => {} ({:.2})\n",
            item.action_summary, item.outcome, item.utility
        ));
    }
    output
}

pub fn render_stats(stats: &MemoryStats) -> String {
    format!(
        "timeline_events: {}\nexperiences: {}\nknowledge: {}\nrules: {}\ntools: {}\n",
        stats.timeline_events, stats.experiences, stats.knowledge, stats.rules, stats.tools
    )
}
