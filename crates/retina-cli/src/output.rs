use retina_memory_sqlite::MemoryStats;
use retina_types::*;

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
            "Files matching '{pattern}' under {}\n{}",
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
