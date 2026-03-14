use crate::controller::WorkerOverview;
use retina_memory_sqlite::MemoryStats;
use retina_types::*;
use serde_json::Value;

pub fn render_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Command(result) => {
            let status = if result.cancelled {
                result
                    .termination
                    .clone()
                    .unwrap_or_else(|| "command cancelled".to_string())
            } else {
                format!("Command completed (exit={:?})", result.exit_code)
            };
            format!(
                "{status}\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )
        }
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

pub fn render_task_state(task_state: &TaskState) -> String {
    let sources = if task_state.working_sources.is_empty() {
        "(none)".to_string()
    } else {
        task_state
            .working_sources
            .iter()
            .map(|source| {
                format!(
                    "- {} [{}|{}|{}] step={} why={}",
                    source.locator,
                    source.kind,
                    source.role,
                    source.status,
                    source.last_used_step,
                    source.why_it_matters
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let artifacts = if task_state.artifact_references.is_empty() {
        "(none)".to_string()
    } else {
        task_state
            .artifact_references
            .iter()
            .map(|reference| {
                format!(
                    "- {} [{}|{}]",
                    reference.locator, reference.kind, reference.status
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let compaction = task_state
        .compaction
        .as_ref()
        .map(|snapshot| {
            let ranking = if snapshot.score_explanations.is_empty() {
                "(none)".to_string()
            } else {
                snapshot
                    .score_explanations
                    .iter()
                    .map(|item| {
                        format!(
                            "- {} {} => {} ({})",
                            item.item_kind, item.locator, item.decision, item.rationale
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!("reason: {}\nranking:\n{}", snapshot.reason, ranking)
        })
        .unwrap_or_else(|| "none".to_string());

    format!(
        "goal: {}\nphase: {}\nstep: {} / {}\nnext: {}\n\nworking_sources:\n{}\n\nartifacts:\n{}\n\ncompaction:\n{}\n",
        task_state.goal.objective,
        task_state.progress.current_phase,
        task_state.progress.current_step,
        task_state.progress.max_steps,
        task_state
            .frontier
            .next_action_hint
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        sources,
        artifacts,
        compaction
    )
}

pub fn render_chat_event(event: &TimelineEvent, debug: bool) -> String {
    if debug {
        return render_timeline_event(event);
    }

    let line = match event.event_type {
        TimelineEventType::TaskReceived => event
            .payload_json
            .get("task")
            .and_then(Value::as_str)
            .map(|task| format!("task: {task}")),
        TimelineEventType::TaskCancelRequested => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("stop requested: {reason}")),
        TimelineEventType::OperatorGuidanceQueued => event
            .payload_json
            .get("guidance")
            .and_then(Value::as_str)
            .map(|guidance| format!("guide: {guidance}")),
        TimelineEventType::ApprovalPromptShown => event
            .payload_json
            .get("action")
            .and_then(Value::as_str)
            .map(|action| format!("approval: {}", humanize_action_label(action))),
        TimelineEventType::ApprovalPromptResolved => event
            .payload_json
            .get("resolution")
            .and_then(Value::as_str)
            .map(|resolution| format!("approval resolved: {resolution}")),
        TimelineEventType::ReasonerCalled => {
            if let Some(action) = event.payload_json.get("action").and_then(Value::as_str) {
                Some(format!("plan: {}", humanize_action_label(action)))
            } else {
                event
                    .payload_json
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
        TimelineEventType::TaskCompacted => event
            .payload_json
            .get("task_state")
            .and_then(|value| serde_json::from_value::<TaskState>(value.clone()).ok())
            .and_then(|task_state| {
                task_state.compaction.as_ref().map(|snapshot| {
                    let kept = snapshot
                        .score_explanations
                        .iter()
                        .filter(|item| item.decision == "keep" || item.decision == "keep_ref")
                        .count();
                    format!("compact: {} (kept {} ranked items)", snapshot.reason, kept)
                })
            })
            .or_else(|| {
                event
                    .payload_json
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|reason| format!("compact: {reason}"))
            }),
        TimelineEventType::TaskStepCompleted => event
            .payload_json
            .get("task_state")
            .and_then(|value| serde_json::from_value::<TaskState>(value.clone()).ok())
            .and_then(|task_state| {
                task_state
                    .working_sources
                    .last()
                    .map(|source| format!("source: {} [{}]", source.locator, source.status))
            }),
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
        output.push_str(&format!(
            "- [{}] {} (confidence {:.2})\n",
            item.category, item.content, item.confidence
        ));
    }
    output.push_str("\nExperiences:\n");
    for item in experiences {
        let task = item
            .metadata
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or("(unknown task)");
        output.push_str(&format!(
            "- task={} | {} => {} ({:.2})\n",
            task, item.action_summary, item.outcome, item.utility
        ));
    }
    output
}

pub fn render_agent_registry(registry: &AgentRegistrySnapshot) -> String {
    let mut output = format!("updated_at: {}\n", registry.updated_at.to_rfc3339());
    output.push_str("active_agents:\n");
    if registry.active_agents.is_empty() {
        output.push_str("- (none)\n");
    } else {
        for agent in &registry.active_agents {
            output.push_str(&format!(
                "- {} [{}] {:?}/{:?} capabilities={}\n",
                agent.agent_id.0,
                agent.domain,
                agent.status,
                agent.lifecycle_phase,
                agent.capabilities.join(", ")
            ));
        }
    }
    output.push_str("archived_agents:\n");
    if registry.archived_agents.is_empty() {
        output.push_str("- (none)\n");
    } else {
        for agent in &registry.archived_agents {
            output.push_str(&format!(
                "- {} [{}] {:?}/{:?} capabilities={}\n",
                agent.agent_id.0,
                agent.domain,
                agent.status,
                agent.lifecycle_phase,
                agent.capabilities.join(", ")
            ));
        }
    }
    output
}

pub fn render_cleanup_report(
    report: &ConsolidationReport,
    keep_events: usize,
    stale_knowledge_days: u64,
    optimize: bool,
) -> String {
    format!(
        "cleanup complete\nkeep_events: {}\nstale_knowledge_days: {}\noptimize: {}\nmerged_knowledge: {}\npromoted_rules: {}\ncompacted_events: {}\ndecayed_knowledge: {}\noptimized: {}\n",
        keep_events,
        stale_knowledge_days,
        optimize,
        report.merged_knowledge,
        report.promoted_rules,
        report.compacted_events,
        report.decayed_knowledge,
        report.optimized
    )
}

pub fn render_worker_overview(overview: &WorkerOverview) -> String {
    let authority_roots = if overview.manifest.authority.accessible_roots.is_empty() {
        "(unscoped)".to_string()
    } else {
        overview
            .manifest
            .authority
            .accessible_roots
            .iter()
            .take(4)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let active_rule_preview = if overview.active_rules.is_empty() {
        "(none)".to_string()
    } else {
        overview
            .active_rules
            .iter()
            .take(5)
            .map(|rule| format!("{} ({:.2})", rule.name, rule.confidence))
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "agent: {} [{}]\nstatus: {:?}/{:?}\nreason: {}\nlast_active: {}\ndb_path: {}\ndb_size_mb: {:.2}\n\ncounts:\n- timeline_events: {}\n- experiences: {}\n- knowledge: {}\n- rules: {}\n- tools: {}\n\nrecent terminal tasks:\n- completed: {}\n- failed: {}\n- cancelled: {}\n- blocked: {}\n\ncompaction:\n- task_compactions: {}\n- cache_reads: {}\n- cache_creations: {}\n\nbudget:\n- max_steps_per_task: {}\n- max_reasoner_calls_per_task: {}\n- max_tokens_per_task: {}\n- idle_archive_after_hours: {}\n\nauthority_roots:\n- {}\n\nactive_rules:\n- {}\n",
        overview.manifest.agent_id.0,
        overview.manifest.domain,
        overview.manifest.status,
        overview.manifest.lifecycle.phase,
        overview
            .manifest
            .lifecycle
            .status_reason
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        overview
            .manifest
            .lifecycle
            .last_active_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "never".to_string()),
        overview.db_path.display(),
        overview.db_size_bytes as f64 / (1024.0 * 1024.0),
        overview.stats.timeline_events,
        overview.stats.experiences,
        overview.stats.knowledge,
        overview.stats.rules,
        overview.stats.tools,
        overview.terminal_tasks.completed,
        overview.terminal_tasks.failed,
        overview.terminal_tasks.cancelled,
        overview.terminal_tasks.blocked,
        overview.compaction_stats.compaction_events,
        overview.compaction_stats.cache_reads,
        overview.compaction_stats.cache_creations,
        overview.manifest.budget.max_steps_per_task,
        overview.manifest.budget.max_reasoner_calls_per_task,
        overview.manifest.budget.max_tokens_per_task,
        overview
            .manifest
            .budget
            .idle_archive_after_hours
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        authority_roots,
        active_rule_preview
    )
}

pub fn render_stats(stats: &MemoryStats) -> String {
    format!(
        "timeline_events: {}\nexperiences: {}\nknowledge: {}\nrules: {}\ntools: {}\n",
        stats.timeline_events, stats.experiences, stats.knowledge, stats.rules, stats.tools
    )
}
