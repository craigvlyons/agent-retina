use crate::controller::WorkerOverview;
use retina_memory_sqlite::MemoryStats;
use retina_runtime::RuntimeTask;
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
            let inspected = state
                .files
                .iter()
                .take(3)
                .map(|item| item.path.display().to_string())
                .collect::<Vec<_>>();
            if inspected.is_empty() {
                format!("Inspection complete for cwd {}", state.cwd.display())
            } else {
                format!("Inspection complete for {}", inspected.join(", "))
            }
        }
        ActionResult::DirectoryListing {
            root,
            entries,
            summary,
        } => format!(
            "Directory listing for {} (files: {}, dirs: {}, hidden: {})\n{}",
            root.display(),
            summary.file_count,
            summary.dir_count,
            summary.hidden_count,
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
            truncated,
            applied_offset,
        } => format!(
            "Paths matching '{pattern}' under {} (offset={}, count={}{}{})\n{}",
            root.display(),
            applied_offset,
            matches.len(),
            if *truncated { ", truncated" } else { "" },
            if *applied_offset > 0 { ", paged" } else { "" },
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
            start_line,
            line_count,
            total_lines,
            ..
        } => format!(
            "Read file {} (start_line={}, line_count={}, total_lines={})\n{}\n{}",
            path.display(),
            start_line,
            line_count,
            total_lines,
            content,
            if *truncated {
                "\n[output truncated]"
            } else {
                ""
            }
        ),
        ActionResult::StructuredData {
            path,
            format,
            headers,
            rows,
            total_rows,
            truncated,
            extraction_method,
        } => format!(
            "Ingested {format} structured data from {}\nmethod: {} | total_rows: {} | sample_rows: {}{}\nheaders: {}\n{}",
            path.display(),
            extraction_method,
            total_rows,
            rows.len(),
            if *truncated {
                " [sample truncated]"
            } else {
                ""
            },
            if headers.is_empty() {
                "(none)".to_string()
            } else {
                headers.join(", ")
            },
            if rows.is_empty() {
                "(no sample rows)".to_string()
            } else {
                rows.iter()
                    .map(|row| format!("- row {}: {}", row.row_number, row.values.join(" | ")))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ),
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            format,
            extraction_method,
            page_range,
            structured_rows_detected,
        } => format!(
            "Extracted {format} text from {}{}\nmethod: {} | structured_rows_detected: {}\n{}\n{}",
            path.display(),
            page_range
                .as_ref()
                .map(|range| format!(" ({})", range.render()))
                .unwrap_or_default(),
            extraction_method,
            structured_rows_detected,
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
            output_mode,
            matches,
            content,
            filenames,
            num_files,
            num_matches,
            truncated,
            applied_offset,
            glob,
            case_insensitive,
        } => {
            let mode_label = match output_mode {
                TextSearchOutputMode::Content => "content",
                TextSearchOutputMode::FilesWithMatches => "files_with_matches",
                TextSearchOutputMode::Count => "count",
            };
            let body = match output_mode {
                TextSearchOutputMode::Content => {
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
                }
                TextSearchOutputMode::FilesWithMatches => {
                    if filenames.is_empty() {
                        "(no files matched)".to_string()
                    } else {
                        filenames
                            .iter()
                            .map(|path| format!("- {}", path.display()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }
                TextSearchOutputMode::Count => content
                    .clone()
                    .unwrap_or_else(|| "(no counted matches)".to_string()),
            };
            format!(
                "Text search for '{query}' under {}{} (mode={}, offset={}, files={}, matches={}{}{}, case_insensitive={})\n{}",
                root.display(),
                glob.as_ref()
                    .map(|value| format!(" [glob={}]", value))
                    .unwrap_or_default(),
                mode_label,
                applied_offset,
                num_files,
                num_matches,
                if *truncated { ", truncated" } else { "" },
                if *applied_offset > 0 { ", paged" } else { "" },
                case_insensitive,
                body
            )
        }
        ActionResult::FileWrite {
            path,
            mutation_kind,
            bytes_written,
            original_hash,
            updated_hash,
            changed_line_count,
            patch_summary,
            preview_excerpt,
            artifact,
            ..
        } => {
            let verb = match mutation_kind {
                FileMutationKind::Create => "Created",
                FileMutationKind::Overwrite => "Overwrote",
                FileMutationKind::Append => "Appended",
                FileMutationKind::ExactEdit => "Edited",
                FileMutationKind::NotebookReplace => "Updated notebook cell(s) in",
                FileMutationKind::NotebookInsert => "Inserted notebook cell(s) into",
                FileMutationKind::NotebookDelete => "Deleted notebook cell(s) from",
            };
            let hash_note = original_hash
                .as_ref()
                .map(|hash| format!("\nprevious_hash: {hash}"))
                .unwrap_or_default();
            let patch_note = patch_summary
                .as_ref()
                .map(|summary| {
                    format!(
                        "\npatch: matched={} replaced={} old=`{}` new=`{}`",
                        summary.matched_occurrences,
                        summary.replaced_occurrences,
                        summary.old_preview,
                        summary.new_preview
                    )
                })
                .unwrap_or_default();
            let preview_note = preview_excerpt
                .as_ref()
                .map(|preview| format!("\npreview: {}", preview))
                .unwrap_or_default();
            let original_note = artifact
                .original_content
                .as_ref()
                .map(|content| format!("\noriginal_chars: {}", content.chars().count()))
                .unwrap_or_default();
            let final_note = format!("\nfinal_chars: {}", artifact.final_content.chars().count());
            format!(
                "{verb} {} ({} byte(s), {} changed line(s))\nupdated_hash: {}{}{}{}{}{}",
                path.display(),
                bytes_written,
                changed_line_count,
                updated_hash,
                hash_note,
                original_note,
                final_note,
                patch_note,
                preview_note
            )
        }
        ActionResult::DelegatedTask(result) => {
            let transcript = result
                .transcript_excerpt
                .as_deref()
                .map(|value| format!("\nchild trace:\n{value}"))
                .unwrap_or_default();
            format!(
                "Delegated child {} finished with {:?} for task {}{}\n{}{}",
                result.agent_id,
                result.status,
                result.task_id,
                result
                    .output_path
                    .as_ref()
                    .map(|path| format!(" output={}", path.display()))
                    .unwrap_or_default(),
                result.summary,
                transcript
            )
        }
        ActionResult::McpResources { server, resources } => {
            let scope = server
                .as_ref()
                .map(|value| format!(" for server {value}"))
                .unwrap_or_else(|| " across configured servers".to_string());
            let body = if resources.is_empty() {
                "(no MCP resources found)".to_string()
            } else {
                resources
                    .iter()
                    .map(|resource| {
                        format!(
                            "- [{}] {} ({})",
                            resource.server, resource.name, resource.uri
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!("MCP resources{scope}\n{body}")
        }
        ActionResult::McpResourceRead(result) => {
            let body = if result.contents.is_empty() {
                "(resource returned no content)".to_string()
            } else {
                result
                    .contents
                    .iter()
                    .map(|item| {
                        let mime = item
                            .mime_type
                            .as_deref()
                            .map(|value| format!(" mime={value}"))
                            .unwrap_or_default();
                        let text = item
                            .text
                            .as_deref()
                            .map(str::to_string)
                            .or_else(|| {
                                item.blob_base64.as_ref().map(|blob| {
                                    format!("[binary content: {} base64 chars]", blob.len())
                                })
                            })
                            .unwrap_or_else(|| "(empty item)".to_string());
                        format!("- {}{}\n{}", item.uri, mime, text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!(
                "Read MCP resource [{}] {}\n{}",
                result.server, result.uri, body
            )
        }
        ActionResult::McpToolCall(result) => {
            let content = result
                .evidence_summary
                .clone()
                .or_else(|| {
                    result.structured_content.as_ref().map(|value| {
                        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
                    })
                })
                .unwrap_or_else(|| result.content_preview.clone());
            format!(
                "Called MCP tool [{}] {}\nerror: {}\n{}",
                result.server,
                result.tool,
                if result.is_error { "yes" } else { "no" },
                content,
            )
        }
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
    let continuation = event
        .payload_json
        .get("continuation_window")
        .and_then(|value| serde_json::from_value::<ActiveContinuationWindow>(value.clone()).ok())
        .map(|window| {
            let source_count = window.transcript.reduced_working_sources().len();
            let artifact_count = window.transcript.reduced_artifact_references().len();
            format!(
                " continuation=transcript:{} refs:{} reannounce:{}/{}/{}",
                window.transcript.len(),
                window.stored_results.len(),
                source_count,
                artifact_count,
                window.reannounced_compacted_results.len()
            )
        })
        .unwrap_or_default();
    format!(
        "[{}] {:?} task={}{}{}\n",
        event.timestamp.to_rfc3339(),
        event.event_type,
        event.task_id,
        event
            .delta_summary
            .as_ref()
            .map(|summary| format!(" delta={summary}"))
            .unwrap_or_default(),
        continuation
    )
}

pub fn render_task_projection(task_state: &TaskState) -> String {
    let sources = if task_state.working_sources.is_empty() {
        "(none)".to_string()
    } else {
        task_state
            .working_sources
            .iter()
            .map(|source| {
                let page_scope = source
                    .page_reference
                    .as_ref()
                    .map(|value| format!("|{}", value))
                    .unwrap_or_default();
                let method = source
                    .extraction_method
                    .as_ref()
                    .map(|value| format!(" method={value}"))
                    .unwrap_or_default();
                let structured = source
                    .structured_summary
                    .as_ref()
                    .map(|value| format!(" {}", value.render()))
                    .unwrap_or_default();
                format!(
                    "- {} [{}|{}|{}{}] step={} why={}{}{}",
                    source.locator,
                    source.kind,
                    source.role,
                    source.status,
                    page_scope,
                    source.last_used_step,
                    source.why_it_matters,
                    method,
                    structured
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
        "goal: {}\nphase: {}\nstep: {} / {}\noutput_written: {}\noutput_verified: {}\n\nworking_sources:\n{}\n\nartifacts:\n{}\n\ncompaction:\n{}\n",
        task_state.goal.objective,
        task_state.progress.current_phase,
        task_state.progress.current_step,
        task_state.progress.max_steps,
        task_state.progress.output_written,
        task_state.progress.output_verified,
        sources,
        artifacts,
        compaction
    )
}

pub fn render_continuation_window(window: &ActiveContinuationWindow) -> String {
    let derived_sources = window.transcript.reduced_working_sources();
    let derived_artifacts = window.transcript.reduced_artifact_references();
    let derived_guidance = window.transcript.latest_next_step_guidance();
    let transcript_units = if window.transcript.is_empty() {
        "(none)".to_string()
    } else {
        window
            .transcript
            .entries()
            .iter()
            .map(|item| item.render())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let stored_result_refs = if window.stored_results.is_empty() {
        "(none)".to_string()
    } else {
        window
            .stored_results
            .entries()
            .iter()
            .map(|item| item.render())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let reannounced_sources = if derived_sources.is_empty() {
        "(none)".to_string()
    } else {
        derived_sources
            .iter()
            .map(WorkingSource::render)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let reannounced_artifacts = if derived_artifacts.is_empty() {
        "(none)".to_string()
    } else {
        derived_artifacts
            .iter()
            .map(ArtifactReference::render)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let reannounced_compacted_results = if window.reannounced_compacted_results.is_empty() {
        "(none)".to_string()
    } else {
        window
            .reannounced_compacted_results
            .iter()
            .map(CompactedResultReference::render)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let boundaries = if window.compaction_boundaries.is_empty() {
        "(none)".to_string()
    } else {
        window
            .compaction_boundaries
            .iter()
            .map(CompactionSnapshot::render)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let guidance = derived_guidance
        .as_ref()
        .map(NextStepGuidance::render)
        .unwrap_or_else(|| "none".to_string());
    let last_transition = window
        .last_transition
        .as_ref()
        .map(|transition| {
            let mut line = transition.reason.clone();
            if let Some(attempt) = transition.attempt {
                line.push_str(&format!(" (attempt {attempt})"));
            }
            if let Some(message) = transition.message.as_deref() {
                line.push_str(&format!(": {message}"));
            }
            line
        })
        .unwrap_or_else(|| "(none)".to_string());
    let search_state_cache = if window.search_state_cache.is_empty() {
        "(none)".to_string()
    } else {
        window
            .search_state_cache
            .iter()
            .map(CachedSearchState::render)
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "objective: {}\nstep: {} / {}\nreasoner_tokens_used: {}\nread_state_cache_entries: {}\nsearch_state_cache_entries: {}\nmax_output_tokens_recovery_count: {}\nhas_attempted_prompt_too_long_compaction: {}\nlast_transition:\n{}\n\nsearch_state_cache:\n{}\n\ntranscript_units:\n{}\n\nstored_result_refs:\n{}\n\nreannounced_sources:\n{}\n\nreannounced_artifacts:\n{}\n\nreannounced_compacted_results:\n{}\n\nnext_step_guidance:\n{}\n\ncompaction_boundaries:\n{}\n",
        window.objective,
        window.current_step,
        window.max_steps,
        window.reasoner_tokens_used,
        window.read_state_cache.len(),
        window.search_state_cache.len(),
        window.max_output_tokens_recovery_count,
        window.has_attempted_prompt_too_long_compaction,
        last_transition,
        search_state_cache,
        transcript_units,
        stored_result_refs,
        reannounced_sources,
        reannounced_artifacts,
        reannounced_compacted_results,
        guidance,
        boundaries
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
                if action.starts_with("respond:") {
                    Some("plan: respond".to_string())
                } else {
                    Some(format!("plan: {}", humanize_action_label(action)))
                }
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
            .map(|action| {
                if action.starts_with("respond:") {
                    "action: respond".to_string()
                } else {
                    format!("action: {}", humanize_action_label(action))
                }
            }),
        TimelineEventType::TaskCompacted => event
            .payload_json
            .get("continuation_window")
            .and_then(|value| {
                serde_json::from_value::<ActiveContinuationWindow>(value.clone()).ok()
            })
            .and_then(|window| render_compaction_line_from_window(&window))
            .or_else(|| {
                event
                    .payload_json
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|reason| format!("compact: {reason}"))
            }),
        TimelineEventType::TaskContextAssembled => event
            .payload_json
            .get("continuation_window")
            .and_then(|value| {
                serde_json::from_value::<ActiveContinuationWindow>(value.clone()).ok()
            })
            .map(|window| {
                let source_count = window.transcript.reduced_working_sources().len();
                let artifact_count = window.transcript.reduced_artifact_references().len();
                format!(
                    "context ready (transcript {}, refs {}, reannounce {}/{}/{})",
                    window.transcript.len(),
                    window.stored_results.len(),
                    source_count,
                    artifact_count,
                    window.reannounced_compacted_results.len()
                )
            }),
        TimelineEventType::ActionResultReceived => event
            .payload_json
            .get("result")
            .and_then(|value| serde_json::from_value::<ActionResult>(value.clone()).ok())
            .and_then(render_observed_result),
        TimelineEventType::ContentReplacementsRecorded => event
            .payload_json
            .get("records")
            .and_then(|value| {
                serde_json::from_value::<Vec<ContentReplacementRecord>>(value.clone()).ok()
            })
            .map(|records| format!("persisted {} exact replacement record(s)", records.len())),
        TimelineEventType::TaskRecoveryContinued => render_recovery_line(event),
        TimelineEventType::TaskContinued => render_continuation_line(event),
        TimelineEventType::TaskStepCompleted => event
            .payload_json
            .get("continuation_window")
            .and_then(|value| {
                serde_json::from_value::<ActiveContinuationWindow>(value.clone()).ok()
            })
            .and_then(|window| render_step_status_from_window(&window)),
        TimelineEventType::ReflectionRequested => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| {
                if reason.starts_with("unsupported operation:") {
                    "retried after tool mismatch".to_string()
                } else {
                    format!("reflection: {reason}")
                }
            }),
        TimelineEventType::TaskFailed => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("failed: {reason}")),
        TimelineEventType::TaskBlocked => event
            .payload_json
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| format!("blocked: {reason}")),
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

fn render_recovery_line(event: &TimelineEvent) -> Option<String> {
    let reason = event.payload_json.get("reason").and_then(Value::as_str)?;
    let attempt = event.payload_json.get("attempt").and_then(Value::as_u64);
    let label = match reason {
        "max_output_tokens_escalate" => {
            let max_tokens = event
                .payload_json
                .get("metadata")
                .and_then(|value| value.get("max_tokens"))
                .and_then(Value::as_u64);
            match max_tokens {
                Some(max_tokens) => {
                    format!("recover: max output tokens via larger budget ({max_tokens})")
                }
                None => "recover: max output tokens via larger budget".to_string(),
            }
        }
        "max_output_tokens_recovery" => match attempt {
            Some(attempt) => format!("recover: max output tokens (attempt {attempt})"),
            None => "recover: max output tokens".to_string(),
        },
        "prompt_too_long_compaction" => "recover: prompt too long via compaction".to_string(),
        other => format!("recover: {other}"),
    };
    Some(label)
}

fn render_continuation_line(event: &TimelineEvent) -> Option<String> {
    let reason = event.payload_json.get("reason").and_then(Value::as_str)?;
    match reason {
        "completion_blocker" => event
            .payload_json
            .get("message")
            .and_then(Value::as_str)
            .map(|message| format!("continue: {message}")),
        "next_turn" => None,
        other => Some(format!("continue: {other}")),
    }
}

fn render_compaction_line_from_window(window: &ActiveContinuationWindow) -> Option<String> {
    window.compaction_boundaries.last().map(|snapshot| {
        let kept = snapshot
            .score_explanations
            .iter()
            .filter(|item| item.decision == "keep" || item.decision == "keep_ref")
            .count();
        let preserved = snapshot.preserved_locators.len();
        let compacted_refs = window.reannounced_compacted_results.len();
        format!(
            "compact: {} (kept {} ranked items, re-announced {} locators, {} compacted refs)",
            snapshot.reason, kept, preserved, compacted_refs
        )
    })
}

fn render_step_status_from_window(window: &ActiveContinuationWindow) -> Option<String> {
    window
        .transcript
        .entries()
        .iter()
        .rev()
        .find_map(|entry| match entry.kind {
            TranscriptUnitKind::TerminalFailure => Some(format!("failed: {}", entry.summary)),
            TranscriptUnitKind::TerminalBlocked => Some(format!("blocked: {}", entry.summary)),
            TranscriptUnitKind::ToolResult | TranscriptUnitKind::FinalResponse => None,
            _ => None,
        })
}

fn render_observed_result(result: ActionResult) -> Option<String> {
    match result {
        ActionResult::Command(result) => {
            let preview = compact_preview(&result.stdout)
                .or_else(|| compact_preview(&result.stderr))
                .unwrap_or_default();
            let suffix = if preview.is_empty() {
                String::new()
            } else {
                format!(" | {preview}")
            };
            Some(format!(
                "observed: {} [executed via run_command]{}",
                result.command, suffix
            ))
        }
        ActionResult::Inspection(state) => {
            let inspected = state
                .files
                .iter()
                .map(|item| item.path.display().to_string())
                .collect::<Vec<_>>();
            let target = match inspected.as_slice() {
                [] => state.cwd.display().to_string(),
                [single] => single.clone(),
                many => format!("{} (+{} more)", many[0], many.len().saturating_sub(1)),
            };
            Some(format!("observed: {} [inspected]", target))
        }
        ActionResult::DirectoryListing { root, .. } => {
            Some(format!("observed: {} [listed]", root.display()))
        }
        ActionResult::FileMatches {
            root,
            pattern,
            matches,
            truncated,
            applied_offset,
        } => Some(format!(
            "observed: {} [matched {} under {}{}{}]",
            pattern,
            matches.len(),
            root.display(),
            if truncated { ", truncated" } else { "" },
            if applied_offset > 0 {
                format!(", offset={applied_offset}")
            } else {
                String::new()
            }
        )),
        ActionResult::FileRead {
            path,
            content,
            truncated,
            ..
        } => Some(format!(
            "observed: {} [read via text_read]{}",
            path.display(),
            preview_suffix(&content, truncated)
        )),
        ActionResult::StructuredData {
            path,
            extraction_method,
            headers,
            ..
        } => {
            let preview = if headers.is_empty() {
                String::new()
            } else {
                format!(" | headers: {}", headers.join(", "))
            };
            Some(format!(
                "observed: {} [structured_read via {}]{}",
                path.display(),
                extraction_method,
                preview
            ))
        }
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            extraction_method,
            ..
        } => Some(format!(
            "observed: {} [extracted via {}]{}",
            path.display(),
            extraction_method,
            preview_suffix(&content, truncated)
        )),
        ActionResult::TextSearch {
            root,
            query,
            output_mode,
            matches,
            filenames,
            num_files,
            num_matches,
            truncated,
            applied_offset,
            glob,
            case_insensitive,
            ..
        } => {
            let mode_summary = match output_mode {
                TextSearchOutputMode::Content => format!("matched {}", matches.len()),
                TextSearchOutputMode::FilesWithMatches => format!("matched {} file(s)", num_files),
                TextSearchOutputMode::Count => {
                    format!(
                        "counted {} match(es) across {} file(s)",
                        num_matches, num_files
                    )
                }
            };
            let preview = match output_mode {
                TextSearchOutputMode::FilesWithMatches if !filenames.is_empty() => format!(
                    " [{}]",
                    filenames
                        .iter()
                        .take(3)
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                _ => String::new(),
            };
            Some(format!(
                "observed: {} [search '{}' {}{}{}{}{}{}]",
                root.display(),
                query,
                mode_summary,
                if truncated { ", truncated" } else { "" },
                if applied_offset > 0 {
                    format!(", offset={applied_offset}")
                } else {
                    String::new()
                },
                glob.as_ref()
                    .map(|value| format!(", glob={value}"))
                    .unwrap_or_default(),
                if case_insensitive {
                    ", case_insensitive"
                } else {
                    ""
                },
                preview
            ))
        }
        ActionResult::FileWrite {
            path,
            mutation_kind,
            preview_excerpt,
            ..
        } => {
            let status = match mutation_kind {
                FileMutationKind::Create => "created",
                FileMutationKind::Overwrite => "written",
                FileMutationKind::Append => "appended",
                FileMutationKind::ExactEdit => "edited",
                FileMutationKind::NotebookReplace
                | FileMutationKind::NotebookInsert
                | FileMutationKind::NotebookDelete => "updated",
            };
            let preview = preview_excerpt
                .as_deref()
                .and_then(compact_preview)
                .map(|value| format!(" | {value}"))
                .unwrap_or_default();
            Some(format!(
                "observed: {} [{} via file_write]{}",
                path.display(),
                status,
                preview
            ))
        }
        ActionResult::DelegatedTask(result) => Some(format!(
            "observed: delegated child {} [{:?}] | {}",
            result.agent_id,
            result.status,
            compact_preview(
                result
                    .transcript_excerpt
                    .as_deref()
                    .unwrap_or(&result.summary)
            )
            .unwrap_or_else(|| result.summary.clone())
        )),
        ActionResult::McpResources { server, resources } => Some(format!(
            "observed: MCP resources [{}] | {} item(s)",
            server.unwrap_or_else(|| "all servers".to_string()),
            resources.len()
        )),
        ActionResult::McpResourceRead(result) => Some(format!(
            "observed: MCP resource {} [{}]",
            result.uri, result.server
        )),
        ActionResult::McpToolCall(result) => {
            let preview = compact_preview(
                result
                    .evidence_summary
                    .as_deref()
                    .unwrap_or(&result.content_preview),
            )
            .map(|value| format!(" | {value}"))
            .unwrap_or_default();
            Some(format!(
                "observed: MCP tool {} [{}]{}",
                result.tool, result.server, preview
            ))
        }
        ActionResult::NoteRecorded { note } => Some(format!("observed: note recorded | {note}")),
        ActionResult::Response { .. } => None,
    }
}

fn preview_suffix(content: &str, truncated: bool) -> String {
    compact_preview(content)
        .map(|value| {
            if truncated {
                format!(" | {}...", value.trim_end_matches("..."))
            } else {
                format!(" | {value}")
            }
        })
        .unwrap_or_default()
}

fn compact_preview(content: &str) -> Option<String> {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    const MAX_CHARS: usize = 120;
    if collapsed.chars().count() <= MAX_CHARS {
        return Some(collapsed);
    }
    let preview = collapsed.chars().take(MAX_CHARS).collect::<String>();
    Some(format!("{preview}..."))
}

fn humanize_action_label(label: &str) -> String {
    if let Some((tool_label, query)) = label.split_once(":query=") {
        if let Some((server, tool)) = parse_mcp_tool_name(tool_label) {
            return format!(
                "mcp tool {}:{} for `{}`",
                server.replace('_', "-"),
                tool.replace('_', " "),
                query
            );
        }
    }
    if let Some((server, tool)) = parse_mcp_tool_name(label) {
        return format!(
            "mcp tool {}:{}",
            server.replace('_', "-"),
            tool.replace('_', " ")
        );
    }
    if let Some(value) = label.strip_prefix("run_command:") {
        return format!("run command `{value}`");
    }
    if let Some(value) = label.strip_prefix("read_file:") {
        return format!("read file {}", value);
    }
    if let Some(value) = label.strip_prefix("extract_document_text:") {
        if let Some((path, pages)) = value.split_once(":pages=") {
            return format!(
                "extract text from {} ({})",
                path,
                pages.replace('-', " to ")
            );
        }
        return format!("extract text from {}", value);
    }
    if let Some(value) = label.strip_prefix("write_file:") {
        return format!("write file {}", value);
    }
    if let Some(value) = label.strip_prefix("edit_file:") {
        return format!("edit file {}", value);
    }
    if let Some(value) = label.strip_prefix("append_file:") {
        return format!("append file {}", value);
    }
    if let Some(value) = label.strip_prefix("edit_notebook:") {
        return format!("edit notebook {}", value);
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
        let mut parts = value.splitn(3, ':');
        let root = parts.next().unwrap_or(".");
        let pattern = parts.next().unwrap_or("*");
        let recursive = parts
            .next()
            .and_then(|value| value.strip_prefix("recursive="))
            .unwrap_or("true");
        return if recursive == "true" {
            format!("find `{pattern}` under {root} recursively")
        } else {
            format!("find `{pattern}` under {root}")
        };
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

pub fn render_mcp_snapshot(snapshot: &McpRegistrySnapshot) -> String {
    if snapshot.servers.is_empty() {
        return "mcp:\n- no configured servers discovered\n".to_string();
    }

    let mut out = String::from("mcp:\n");
    for server in &snapshot.servers {
        if let Some(error) = &server.error {
            out.push_str(&format!("- {} [error]\n  error: {}\n", server.name, error));
            continue;
        }

        out.push_str(&format!(
            "- {} [connected]\n  tools: {}\n  resources: {}\n",
            server.name,
            server.tools.len(),
            server.resources.len()
        ));
        if !server.tools.is_empty() {
            let tool_names = server
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("  tool_names: {}\n", tool_names));
        }
        if !server.resources.is_empty() {
            let resource_names = server
                .resources
                .iter()
                .take(5)
                .map(|resource| resource.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("  resource_names: {}\n", resource_names));
        }
    }
    out
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
    let allowed_tools = if overview.manifest.allowed_tools.is_empty() {
        "(inherited from authority)".to_string()
    } else {
        overview.manifest.allowed_tools.join(", ")
    };
    let denied_tools = if overview.manifest.denied_tools.is_empty() {
        "(none)".to_string()
    } else {
        overview.manifest.denied_tools.join(", ")
    };
    let required_mcp_servers = if overview.manifest.required_mcp_servers.is_empty() {
        "(none)".to_string()
    } else {
        overview.manifest.required_mcp_servers.join(", ")
    };
    let role_prompt = overview
        .manifest
        .role_prompt
        .clone()
        .unwrap_or_else(|| "(none)".to_string());
    let initial_prompt = overview
        .manifest
        .initial_prompt
        .clone()
        .unwrap_or_else(|| "(none)".to_string());
    let agent_model = overview
        .manifest
        .model_id
        .clone()
        .unwrap_or_else(|| "(inherit runtime default)".to_string());
    let runtime_tasks = render_runtime_tasks_inline(&overview.runtime_tasks);

    format!(
        "agent: {} [{}]\nstatus: {:?}/{:?}\nreason: {}\nlast_active: {}\ndb_path: {}\ndb_size_mb: {:.2}\n\ncounts:\n- timeline_events: {}\n- experiences: {}\n- knowledge: {}\n- rules: {}\n- tools: {}\n\nrecent terminal tasks:\n- completed: {}\n- failed: {}\n- cancelled: {}\n- blocked: {}\n\nrecovery transitions:\n- total: {}\n- max_output_tokens_escalate: {}\n- max_output_tokens_recovery: {}\n- prompt_too_long_compaction: {}\n\nruntime_tasks:\n{}\ncompaction:\n- task_compactions: {}\n- cache_reads: {}\n- cache_creations: {}\n\nclaude_runtime:\n- model: {}\n- prompt_caching: {}\n- context_editing: {}\n- tool_result_trigger_tokens: {}\n- server_compaction_requested: {}\n- server_compaction_supported: {}\n- server_compaction_effective: {}\n- compaction_trigger_tokens: {}\n\nbudget:\n- max_steps_per_task: {}\n- max_reasoner_calls_per_task: {}\n- max_tokens_per_task: {}\n- idle_archive_after_hours: {}\n\nagent_model:\n{}\n\nrole_prompt:\n{}\n\ninitial_prompt:\n{}\n\ntool_scope:\n- allowed: {}\n- denied: {}\n- required_mcp_servers: {}\n\nauthority_roots:\n- {}\n\nactive_rules:\n- {}\n",
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
        overview.recovery_stats.total,
        overview.recovery_stats.max_output_tokens_escalate,
        overview.recovery_stats.max_output_tokens_recovery,
        overview.recovery_stats.prompt_too_long_compaction,
        runtime_tasks,
        overview.compaction_stats.compaction_events,
        overview.compaction_stats.cache_reads,
        overview.compaction_stats.cache_creations,
        overview.claude_runtime.model_id,
        overview.claude_runtime.prompt_caching_enabled,
        overview.claude_runtime.context_editing_enabled,
        overview.claude_runtime.tool_result_trigger_tokens,
        overview.claude_runtime.server_side_compaction_requested,
        overview.claude_runtime.server_side_compaction_supported,
        overview.claude_runtime.server_side_compaction_effective,
        overview.claude_runtime.compaction_trigger_tokens,
        overview.manifest.budget.max_steps_per_task,
        overview.manifest.budget.max_reasoner_calls_per_task,
        overview.manifest.budget.max_tokens_per_task,
        overview
            .manifest
            .budget
            .idle_archive_after_hours
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        agent_model,
        role_prompt,
        initial_prompt,
        allowed_tools,
        denied_tools,
        required_mcp_servers,
        authority_roots,
        active_rule_preview
    )
}

pub fn render_runtime_tasks(tasks: &[RuntimeTask]) -> String {
    if tasks.is_empty() {
        return "runtime_tasks:\n- (none)\n".to_string();
    }
    format!("runtime_tasks:\n{}", render_runtime_tasks_inline(tasks))
}

fn render_runtime_tasks_inline(tasks: &[RuntimeTask]) -> String {
    if tasks.is_empty() {
        return "- (none)\n".to_string();
    }
    tasks
        .iter()
        .map(|task| {
            let terminal = if task.status.is_terminal() {
                " terminal"
            } else {
                ""
            };
            let output = task
                .output_path
                .as_ref()
                .map(|path| format!(" output={}", path.display()))
                .unwrap_or_default();
            let parent = task
                .parent_task_id
                .as_ref()
                .map(|parent| format!(" parent={parent}"))
                .unwrap_or_default();
            let summary = task
                .progress_summary
                .as_deref()
                .unwrap_or("(no progress summary)");
            format!(
                "- {} [{:?}|{:?}{}] owner={}{} updated={}{} :: {}",
                task.task_id,
                task.task_kind,
                task.status,
                terminal,
                task.owner_agent_id,
                parent,
                task.last_activity.to_rfc3339(),
                output,
                summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

pub fn render_stats(stats: &MemoryStats) -> String {
    format!(
        "timeline_events: {}\nexperiences: {}\nknowledge: {}\nrules: {}\ntools: {}\n",
        stats.timeline_events, stats.experiences, stats.knowledge, stats.rules, stats.tools
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn action_result_received_renders_file_read_observation() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::ActionResultReceived,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::FileRead {
                    path: "/tmp/plan.pdf".into(),
                    content: "panel schedule and electrical notes".to_string(),
                    truncated: false,
                    start_line: 1,
                    line_count: 1,
                    total_lines: 1,
                    total_bytes: 35,
                    read_bytes: 35,
                }
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "observed: /tmp/plan.pdf [read via text_read] | panel schedule and electrical notes\n"
        );
    }

    #[test]
    fn action_result_received_renders_file_write_preview() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::ActionResultReceived,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::FileWrite {
                    path: "/Users/macc/Desktop/transcript/notes_summary.md".into(),
                    mutation_kind: FileMutationKind::Create,
                    bytes_written: 10,
                    created: true,
                    overwritten: false,
                    appended: false,
                    original_hash: None,
                    updated_hash: "hash".to_string(),
                    changed_line_count: 2,
                    patch_summary: None,
                    preview_excerpt: Some("# Summary of Desktop/Notes Files".to_string()),
                    artifact: FileArtifactPayload {
                        original_content: None,
                        final_content: "# Summary of Desktop/Notes Files".to_string(),
                    },
                }
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "observed: /Users/macc/Desktop/transcript/notes_summary.md [created via file_write] | # Summary of Desktop/Notes Files\n"
        );
    }

    #[test]
    fn action_result_received_renders_inspected_path_not_cwd() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::ActionResultReceived,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::Inspection(WorldState {
                    cwd: "/Users/macc/projects/personal/agent-retina".into(),
                    files: vec![PathState {
                        path: "/Users/macc/Desktop/transcriptions".into(),
                        exists: true,
                        size: Some(0),
                        modified_at: None,
                        content_hash: None,
                    }],
                    last_command: None,
                    notes: Vec::new(),
                })
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "observed: /Users/macc/Desktop/transcriptions [inspected]\n"
        );
    }

    #[test]
    fn action_result_received_prefers_latest_command_result_without_stale_blockers() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::ActionResultReceived,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "result": ActionResult::Command(CommandResult {
                    command: "ps aux | grep -i docker | grep -v grep".to_string(),
                    cwd: "/Users/macc/projects/personal/agent-retina".into(),
                    stdout: "Docker helper still running".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                    duration_ms: 12,
                    cancelled: false,
                    termination: None,
                    observed_paths: vec![],
                })
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "observed: ps aux | grep -i docker | grep -v grep [executed via run_command] | Docker helper still running\n"
        );
    }

    #[test]
    fn task_step_completed_is_silent_in_normal_chat_mode() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskStepCompleted,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": ActiveContinuationWindow::default()
            }),
            delta_summary: None,
        };

        assert_eq!(render_chat_event(&event, false), "");
    }

    #[test]
    fn task_step_completed_surfaces_failed_step_summary() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskStepCompleted,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": ActiveContinuationWindow {
                    objective: "search".to_string(),
                    current_step: 2,
                    max_steps: 4,
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 2,
                        step: 2,
                        kind: TranscriptUnitKind::TerminalFailure,
                        summary: "mcp-tool://brave/brave_web_search is MCP output, not a filesystem path".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    }]),
                    ..ActiveContinuationWindow::default()
                }
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "failed: mcp-tool://brave/brave_web_search is MCP output, not a filesystem path\n"
        );
    }

    #[test]
    fn task_step_completed_prefers_continuation_window_terminal_status() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskStepCompleted,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": ActiveContinuationWindow {
                    objective: "search".to_string(),
                    current_step: 2,
                    max_steps: 4,
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 2,
                        step: 2,
                        kind: TranscriptUnitKind::TerminalBlocked,
                        summary: "repeated the same step without new evidence".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                    compaction_snapshot: None,
                    }]),
                    ..ActiveContinuationWindow::default()
                }
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "blocked: repeated the same step without new evidence\n"
        );
    }

    #[test]
    fn task_blocked_renders_blocked_reason() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskBlocked,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "repeated the same step without new evidence"
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "blocked: repeated the same step without new evidence\n"
        );
    }

    #[test]
    fn task_recovery_continued_renders_recovery_reason() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskRecoveryContinued,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "max_output_tokens_recovery",
                "attempt": 2,
                "message": "Output token limit hit. Resume directly."
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "recover: max output tokens (attempt 2)\n"
        );
    }

    #[test]
    fn task_recovery_continued_renders_max_output_tokens_escalation_reason() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskRecoveryContinued,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "max_output_tokens_escalate",
                "attempt": 1,
                "message": "Retrying the same request with a larger output token budget.",
                "metadata": { "max_tokens": 64000 }
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "recover: max output tokens via larger budget (64000)\n"
        );
    }

    #[test]
    fn task_recovery_continued_renders_prompt_too_long_compaction_reason() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskRecoveryContinued,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "prompt_too_long_compaction",
                "attempt": 1,
                "message": "Context limit hit. Continue from the compacted thread only."
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "recover: prompt too long via compaction\n"
        );
    }

    #[test]
    fn task_continued_renders_completion_blocker_reason() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskContinued,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "completion_blocker",
                "message": "still missing one source from the batch input"
            }),
            delta_summary: None,
        };

        assert_eq!(
            render_chat_event(&event, false),
            "continue: still missing one source from the batch input\n"
        );
    }

    #[test]
    fn task_continued_hides_next_turn_noise_in_chat_mode() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            event_type: TimelineEventType::TaskContinued,
            timestamp: Utc::now(),
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            duration_ms: None,
            payload_json: json!({
                "reason": "next_turn",
                "message": "continuing after non-terminal tool progress"
            }),
            delta_summary: None,
        };

        assert_eq!(render_chat_event(&event, false), "");
    }

    #[test]
    fn task_context_assembled_renders_continuation_window_counts() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: chrono::Utc::now(),
            event_type: TimelineEventType::TaskContextAssembled,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: serde_json::json!({
                "continuation_window": ActiveContinuationWindow {
                    objective: "search".to_string(),
                    current_step: 1,
                    max_steps: 4,
                    reasoner_tokens_used: 0,
                    max_output_tokens_recovery_count: 0,
                    has_attempted_prompt_too_long_compaction: false,
                    last_transition: None,
                    read_state_cache: Vec::new(),
                    search_state_cache: Vec::new(),
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 1,
                        step: 1,
                        kind: TranscriptUnitKind::TaskMessage,
                        summary: "search".to_string(),
                        result_ref_id: None,
                        primary_locator: None,
                        evidence_refs: Vec::new(),
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                    compaction_snapshot: None,
                    }]),
                    stored_results: StoredResultLedger::from_entries(vec![StoredResultReference {
                        result_id: "result-1-1".to_string(),
                        source_transcript_ordinal: 1,
                        step: 1,
                        result_type: "mcp_tool_call".to_string(),
                        primary_locator: None,
                        preview_excerpt: "preview".to_string(),
                        persisted_path: "/tmp/result.json".to_string(),
                    }]),
                    content_replacements: ContentReplacementState::from_continuation(
                        &StoredResultLedger::from_entries(vec![StoredResultReference {
                            result_id: "result-1-1".to_string(),
                            source_transcript_ordinal: 1,
                            step: 1,
                            result_type: "mcp_tool_call".to_string(),
                            primary_locator: None,
                            preview_excerpt: "preview".to_string(),
                            persisted_path: "/tmp/result.json".to_string(),
                        }]),
                        &[],
                    ),
                    reannounced_sources: Vec::new(),
                    reannounced_artifacts: Vec::new(),
                    next_step_guidance: None,
                    compaction_boundaries: Vec::new(),
                    reannounced_compacted_results: Vec::new(),
                }
            }),
        };

        assert_eq!(
            render_chat_event(&event, false),
            "context ready (transcript 1, refs 1, reannounce 0/0/0)\n"
        );
    }

    #[test]
    fn render_continuation_window_prefers_transcript_derived_context() {
        let rendered = render_continuation_window(&ActiveContinuationWindow {
            objective: "inspect gabactl".to_string(),
            current_step: 3,
            max_steps: 8,
            reasoner_tokens_used: 640,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            max_output_tokens_recovery_count: 2,
            has_attempted_prompt_too_long_compaction: true,
            last_transition: Some(ContinuationTransition {
                reason: "max_output_tokens_escalate".to_string(),
                attempt: Some(1),
                message: Some(
                    "Retrying the same request with a larger output token budget.".to_string(),
                ),
                metadata: serde_json::Value::Null,
            }),
            transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                ordinal: 1,
                step: 3,
                kind: TranscriptUnitKind::CarryoverMessage,
                summary: "source reminder: /tmp/gabactl [tool|read]".to_string(),
                result_ref_id: None,
                primary_locator: Some("/tmp/gabactl".to_string()),
                evidence_refs: vec!["/tmp/gabactl".to_string()],
                working_sources: vec![WorkingSource {
                    kind: "tool".to_string(),
                    locator: "/tmp/gabactl".to_string(),
                    role: "validated".to_string(),
                    status: "read".to_string(),
                    why_it_matters: "validated external tool path".to_string(),
                    last_used_step: 3,
                    evidence_refs: vec!["/tmp/gabactl".to_string()],
                    page_reference: None,
                    extraction_method: None,
                    structured_summary: None,
                    preview_excerpt: Some("validated gabactl binary".to_string()),
                }],
                artifact_references: vec![ArtifactReference {
                    kind: "file".to_string(),
                    locator: "/tmp/result.json".to_string(),
                    status: "written".to_string(),
                }],
                next_step_guidance: Some(NextStepGuidance {
                    directive: NextStepDirective::AnswerFromEvidence,
                    reason: "the tool path is already validated".to_string(),
                    based_on_action: None,
                    evidence_locator: Some("/tmp/gabactl".to_string()),
                    preferred_search_family: None,
                    suggested_query: None,
                }),
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            }]),
            reannounced_sources: Vec::new(),
            reannounced_artifacts: Vec::new(),
            next_step_guidance: None,
            ..ActiveContinuationWindow::default()
        });

        assert!(rendered.contains("/tmp/gabactl"));
        assert!(rendered.contains("/tmp/result.json"));
        assert!(rendered.contains("the tool path is already validated"));
        assert!(rendered.contains("reasoner_tokens_used: 640"));
        assert!(rendered.contains("max_output_tokens_recovery_count: 2"));
        assert!(rendered.contains("has_attempted_prompt_too_long_compaction: true"));
        assert!(rendered.contains("max_output_tokens_escalate (attempt 1)"));
    }

    #[test]
    fn task_compacted_prefers_continuation_window_summary() {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: SessionId::new(),
            task_id: TaskId::new(),
            agent_id: AgentId::new(),
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskCompacted,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({
                "continuation_window": ActiveContinuationWindow {
                    objective: "search".to_string(),
                    current_step: 3,
                    max_steps: 4,
                    reasoner_tokens_used: 0,
                    max_output_tokens_recovery_count: 0,
                    has_attempted_prompt_too_long_compaction: false,
                    last_transition: None,
                    read_state_cache: Vec::new(),
                    search_state_cache: Vec::new(),
                    compaction_boundaries: vec![CompactionSnapshot {
                        boundary_id: 1,
                        compacted_at_step: 3,
                        reason: "large tool result".to_string(),
                        score_explanations: vec![
                            CompactionScoreExplanation {
                                item_kind: "source".to_string(),
                                locator: "a.md".to_string(),
                                decision: "keep".to_string(),
                                rationale: "important".to_string(),
                            },
                            CompactionScoreExplanation {
                                item_kind: "source".to_string(),
                                locator: "b.md".to_string(),
                                decision: "compact".to_string(),
                                rationale: "less important".to_string(),
                            },
                        ],
                        preserved_locators: vec!["a.md".to_string()],
                        active_window_summary: "summary".to_string(),
                        last_result_continuation: None,
                        compacted_results: vec![CompactedResultReference {
                            boundary_id: 1,
                            result_type: "directory_listing".to_string(),
                            locator: Some("/tmp".to_string()),
                            preview_excerpt: "preview".to_string(),
                            continuation: None,
                            persisted_path: Some("/tmp/boundary-1.json".to_string()),
                        }],
                    }],
                    reannounced_compacted_results: vec![CompactedResultReference {
                        boundary_id: 1,
                        result_type: "directory_listing".to_string(),
                        locator: Some("/tmp".to_string()),
                        preview_excerpt: "preview".to_string(),
                        continuation: None,
                        persisted_path: Some("/tmp/boundary-1.json".to_string()),
                    }],
                    ..ActiveContinuationWindow::default()
                }
            }),
        };

        assert_eq!(
            render_chat_event(&event, false),
            "compact: large tool result (kept 1 ranked items, re-announced 1 locators, 1 compacted refs)\n"
        );
    }
}
