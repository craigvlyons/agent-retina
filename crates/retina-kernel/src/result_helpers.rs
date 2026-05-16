use super::action_label;
use retina_types::*;
use std::path::{Path, PathBuf};

pub(crate) fn summarize_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Command(command) => format!(
            "command {} with exit {:?}{}",
            if command.success {
                "succeeded"
            } else {
                "failed"
            },
            command.exit_code,
            if command.observed_paths.is_empty() {
                String::new()
            } else {
                format!(
                    " (changed: {})",
                    preview_paths(command.observed_paths.clone())
                )
            }
        ),
        ActionResult::Inspection(world) => format!("inspected {} path(s)", world.files.len()),
        ActionResult::DirectoryListing {
            root,
            entries,
            summary,
        } => format!(
            "listed {} entr{} under {} (files={}, dirs={}, hidden={}) [{}]",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
            root.display(),
            summary.file_count,
            summary.dir_count,
            summary.hidden_count,
            preview_paths(entries.iter().map(|entry| entry.path.clone()).collect())
        ),
        ActionResult::FileMatches {
            pattern, matches, ..
        } => format!(
            "found {} match{} for {} [{}]",
            matches.len(),
            if matches.len() == 1 { "" } else { "es" },
            pattern,
            preview_paths(matches.clone())
        ),
        ActionResult::FileRead {
            path,
            content,
            truncated,
        } => format!(
            "read {} ({} chars{}): {}",
            path.display(),
            content.chars().count(),
            if *truncated { ", truncated" } else { "" },
            preview_text(content, 120)
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
            "ingested {} structured data from {} via {} (headers={}, sample_rows={} of {}{}): {}",
            format,
            path.display(),
            extraction_method,
            headers.join(", "),
            rows.len(),
            total_rows,
            if *truncated { ", truncated" } else { "" },
            preview_structured_rows(rows, 2)
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
            "extracted {} text from {}{} via {} ({} chars{}, structured_rows_detected={}): {}",
            format,
            path.display(),
            page_range
                .as_ref()
                .map(|range| format!(" ({})", range.render()))
                .unwrap_or_default(),
            extraction_method,
            content.chars().count(),
            if *truncated { ", truncated" } else { "" },
            structured_rows_detected,
            preview_text(content, 120)
        ),
        ActionResult::TextSearch { query, matches, .. } => format!(
            "found {} text match{} for {} [{}]",
            matches.len(),
            if matches.len() == 1 { "" } else { "es" },
            query,
            preview_search_matches(matches)
        ),
        ActionResult::McpResources { server, resources } => format!(
            "listed {} MCP resource{}{}",
            resources.len(),
            if resources.len() == 1 { "" } else { "s" },
            server
                .as_ref()
                .map(|name| format!(" from {name}"))
                .unwrap_or_default()
        ),
        ActionResult::McpResourceRead(result) => format!(
            "read MCP resource {} from {} ({} content item{})",
            result.uri,
            result.server,
            result.contents.len(),
            if result.contents.len() == 1 { "" } else { "s" }
        ),
        ActionResult::McpToolCall(result) => format!(
            "called MCP tool {}/{}{}{}",
            result.server,
            result.tool,
            if result.is_error { " with error" } else { "" },
            summarize_mcp_tool_signal(result)
        ),
        ActionResult::FileWrite {
            path,
            mutation_kind,
            bytes_written,
            patch_summary,
            preview_excerpt,
            artifact,
            ..
        } => {
            let verb = match mutation_kind {
                FileMutationKind::Create => "created",
                FileMutationKind::Overwrite => "overwrote",
                FileMutationKind::Append => "appended to",
                FileMutationKind::ExactEdit => "edited",
                FileMutationKind::NotebookReplace => "updated notebook",
                FileMutationKind::NotebookInsert => "inserted notebook cells into",
                FileMutationKind::NotebookDelete => "deleted notebook cells from",
            };
            let patch = patch_summary
                .as_ref()
                .map(|summary| {
                    format!(
                        " [{} match(es), {} replacement(s)]",
                        summary.matched_occurrences, summary.replaced_occurrences
                    )
                })
                .unwrap_or_default();
            let preview = preview_excerpt
                .as_ref()
                .map(|value| format!(": {}", preview_text(value, 120)))
                .unwrap_or_default();
            let content_note = if artifact.final_content.is_empty() {
                String::new()
            } else {
                format!(
                    " [artifact chars={}]",
                    artifact.final_content.chars().count()
                )
            };
            format!(
                "{verb} {} ({} bytes){patch}{content_note}{preview}",
                path.display(),
                bytes_written,
            )
        }
        ActionResult::DelegatedTask(result) => format!(
            "delegated child {} finished with {:?}: {}",
            result.agent_id, result.status, result.summary
        ),
        ActionResult::NoteRecorded { note } => format!("recorded note: {}", note),
        ActionResult::Response { message } => format!("responded: {}", message),
    }
}

pub(crate) fn compact_action_result_for_context(
    result: &ActionResult,
) -> serde_json::Result<String> {
    let compact = match result {
        ActionResult::Command(command) => serde_json::json!({
            "type": "command",
            "command": command.command,
            "cwd": command.cwd,
            "success": command.success,
            "exit_code": command.exit_code,
            "cancelled": command.cancelled,
            "termination": command.termination,
            "observed_paths": command.observed_paths,
            "stdout": preview_text(&command.stdout, 2000),
            "stderr": preview_text(&command.stderr, 1000),
        }),
        ActionResult::Inspection(world) => serde_json::json!({
            "type": "inspection",
            "cwd": world.cwd,
            "paths": world
                .files
                .iter()
                .take(8)
                .map(|path| path.path.display().to_string())
                .collect::<Vec<_>>(),
        }),
        ActionResult::DirectoryListing {
            root,
            entries,
            summary,
        } => serde_json::json!({
            "type": "directory_listing",
            "root": root,
            "count": entries.len(),
            "summary": {
                "total_entries": summary.total_entries,
                "file_count": summary.file_count,
                "dir_count": summary.dir_count,
                "hidden_count": summary.hidden_count,
                "sample_names": summary.sample_names,
            },
            "entries": entries
                .iter()
                .take(12)
                .map(|entry| serde_json::json!({
                    "path": entry.path,
                    "is_dir": entry.is_dir
                }))
                .collect::<Vec<_>>(),
        }),
        ActionResult::FileMatches {
            root,
            pattern,
            matches,
        } => serde_json::json!({
            "type": "file_matches",
            "root": root,
            "pattern": pattern,
            "count": matches.len(),
            "matches": matches.iter().take(12).collect::<Vec<_>>(),
        }),
        ActionResult::FileRead {
            path,
            content,
            truncated,
        } => serde_json::json!({
            "type": "file_read",
            "path": path,
            "truncated": truncated,
            "content": preview_text(content, 8000),
        }),
        ActionResult::StructuredData {
            path,
            format,
            headers,
            rows,
            total_rows,
            truncated,
            extraction_method,
        } => serde_json::json!({
            "type": "structured_data",
            "path": path,
            "format": format,
            "headers": headers,
            "rows": rows,
            "total_rows": total_rows,
            "truncated": truncated,
            "extraction_method": extraction_method,
        }),
        ActionResult::DocumentText {
            path,
            content,
            truncated,
            format,
            extraction_method,
            page_range,
            structured_rows_detected,
        } => serde_json::json!({
            "type": "document_text",
            "path": path,
            "format": format,
            "extraction_method": extraction_method,
            "page_range": page_range,
            "structured_rows_detected": structured_rows_detected,
            "truncated": truncated,
            "content": preview_text(content, 8000),
        }),
        ActionResult::TextSearch {
            root,
            query,
            matches,
        } => serde_json::json!({
            "type": "text_search",
            "root": root,
            "query": query,
            "count": matches.len(),
            "matches": matches
                .iter()
                .take(8)
                .map(|item| serde_json::json!({
                    "path": item.path,
                    "line_number": item.line_number,
                    "line": preview_text(&item.line, 180),
                }))
                .collect::<Vec<_>>(),
        }),
        ActionResult::McpResources { server, resources } => serde_json::json!({
            "type": "mcp_resources",
            "server": server,
            "count": resources.len(),
            "resources": resources.iter().take(12).collect::<Vec<_>>(),
        }),
        ActionResult::McpResourceRead(result) => serde_json::json!({
            "type": "mcp_resource_read",
            "server": result.server,
            "uri": result.uri,
            "contents": result.contents,
        }),
        ActionResult::McpToolCall(result) => serde_json::json!({
            "type": "mcp_tool_call",
            "server": result.server,
            "tool": result.tool,
            "is_error": result.is_error,
            "content_preview": preview_text(&result.content_preview, 4000),
            "structured_content": result.structured_content,
        }),
        ActionResult::FileWrite {
            path,
            mutation_kind,
            bytes_written,
            created,
            overwritten,
            appended,
            original_hash,
            updated_hash,
            changed_line_count,
            patch_summary,
            preview_excerpt,
            artifact,
        } => serde_json::json!({
            "type": "file_write",
            "path": path,
            "mutation_kind": mutation_kind,
            "bytes_written": bytes_written,
            "created": created,
            "overwritten": overwritten,
            "appended": appended,
            "original_hash": original_hash,
            "updated_hash": updated_hash,
            "changed_line_count": changed_line_count,
            "patch_summary": patch_summary,
            "preview_excerpt": preview_excerpt,
            "artifact": {
                "original_content": artifact.original_content.as_ref().map(|content| preview_text(content, 4000)),
                "final_content": preview_text(&artifact.final_content, 8000),
            },
        }),
        ActionResult::DelegatedTask(result) => serde_json::json!({
            "type": "delegated_task",
            "agent_id": result.agent_id,
            "task_id": result.task_id,
            "parent_task_id": result.parent_task_id,
            "status": result.status,
            "summary": preview_text(&result.summary, 200),
            "transcript_excerpt": result
                .transcript_excerpt
                .as_ref()
                .map(|value| preview_text(value, 800)),
            "output_path": result.output_path,
        }),
        ActionResult::NoteRecorded { note } => serde_json::json!({
            "type": "note",
            "note": preview_text(note, 200),
        }),
        ActionResult::Response { message } => serde_json::json!({
            "type": "response",
            "message": preview_text(message, 200),
        }),
    };
    serde_json::to_string(&compact)
}

pub(crate) fn repeated_step_signature(action: &Action, result: &ActionResult) -> Option<String> {
    if matches!(action, Action::Respond { .. } | Action::RecordNote { .. }) {
        return None;
    }

    if let (Action::RunCommand { command, .. }, ActionResult::Command(outcome)) = (action, result) {
        return Some(format!(
            "command_family:{}::success={}::exit={:?}::observed_paths={}",
            normalize_command_family(command),
            outcome.success,
            outcome.exit_code,
            outcome.observed_paths.len()
        ));
    }

    Some(format!(
        "{}::{}",
        action_label(action),
        summarize_action_result(result)
    ))
}

pub(crate) fn compact_last_result_for_compacted_context(
    last_result: &str,
) -> serde_json::Result<String> {
    let value: serde_json::Value = serde_json::from_str(last_result)?;
    let compact = match value.get("type").and_then(serde_json::Value::as_str) {
        Some("file_read") => serde_json::json!({
            "type": "file_read",
            "path": value.get("path"),
            "truncated": value.get("truncated"),
            "content_preview": value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(|text| preview_text(text, 240)),
            "continuation": "reopen file by path from task_state artifact refs if more detail is needed"
        }),
        Some("document_text") => serde_json::json!({
            "type": "document_text",
            "path": value.get("path"),
            "format": value.get("format"),
            "extraction_method": value.get("extraction_method"),
            "page_range": value.get("page_range"),
            "structured_rows_detected": value.get("structured_rows_detected"),
            "truncated": value.get("truncated"),
            "content_preview": value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(|text| preview_text(text, 240)),
            "continuation": "reopen extracted document by path from task_state artifact refs if more detail is needed"
        }),
        Some("structured_data") => serde_json::json!({
            "type": "structured_data",
            "path": value.get("path"),
            "format": value.get("format"),
            "headers": value.get("headers"),
            "rows": value.get("rows"),
            "total_rows": value.get("total_rows"),
            "truncated": value.get("truncated"),
            "extraction_method": value.get("extraction_method"),
            "continuation": "reopen structured data by path from task_state artifact refs if more rows are needed"
        }),
        Some("text_search") => serde_json::json!({
            "type": "text_search",
            "root": value.get("root"),
            "query": value.get("query"),
            "count": value.get("count"),
            "matches": value.get("matches"),
            "continuation": "use task_state working sources and artifact refs for exact evidence"
        }),
        Some("file_matches") => serde_json::json!({
            "type": "file_matches",
            "root": value.get("root"),
            "pattern": value.get("pattern"),
            "count": value.get("count"),
            "matches": value.get("matches"),
            "continuation": "choose from task_state candidate sources instead of re-searching"
        }),
        Some("directory_listing") => serde_json::json!({
            "type": "directory_listing",
            "root": value.get("root"),
            "count": value.get("count"),
            "entries": value.get("entries"),
            "continuation": "use task_state candidate sources instead of replaying the full listing"
        }),
        _ => value,
    };
    serde_json::to_string(&compact)
}

pub(crate) fn summarize_verified_facts(
    working_sources: &[WorkingSource],
    references: &[ArtifactReference],
) -> Vec<String> {
    let mut facts = Vec::new();

    for source in prioritized_working_sources(working_sources)
        .into_iter()
        .take(5)
    {
        let fact = match (source.role.as_str(), source.status.as_str()) {
            ("authoritative", "read") => format!("authoritative file read from {}", source.locator),
            ("authoritative", "excerpted") => {
                format!(
                    "authoritative document text extracted from {}",
                    source.locator
                )
            }
            ("authoritative", "ingested") => {
                format!(
                    "authoritative structured data ingested from {}",
                    source.locator
                )
            }
            ("generated", status) => format!("produced artifact {} ({status})", source.locator),
            (_, "matched") => format!("candidate source identified at {}", source.locator),
            (_, "matched_text") => format!("text evidence identified in {}", source.locator),
            (_, "listed") => format!("directory explored at {}", source.locator),
            (_, "inspected") => format!("path inspected at {}", source.locator),
            _ => format!("{} {} [{}]", source.status, source.locator, source.role),
        };
        push_unique(&mut facts, fact);
    }

    for reference in prioritized_artifact_references(references)
        .into_iter()
        .take(5)
    {
        let fact = match reference.status.as_str() {
            "read" => format!("exact evidence kept for {}", reference.locator),
            "structured_read" => {
                format!("exact structured evidence kept for {}", reference.locator)
            }
            "extracted" => format!("exact extracted evidence kept for {}", reference.locator),
            "matched" => format!(
                "candidate artifact reference kept for {}",
                reference.locator
            ),
            "searched" => format!("search evidence kept for {}", reference.locator),
            "created" | "written" | "overwritten" | "appended" | "command_changed" => {
                format!(
                    "output or changed artifact tracked at {}",
                    reference.locator
                )
            }
            _ => format!("{} {}", reference.status, reference.locator),
        };
        push_unique(&mut facts, fact);
    }

    facts.into_iter().take(8).collect()
}

pub(crate) fn prioritized_working_sources(sources: &[WorkingSource]) -> Vec<WorkingSource> {
    let mut sources = sources.to_vec();
    sources.sort_by(|left, right| {
        working_source_rank(right)
            .cmp(&working_source_rank(left))
            .then_with(|| right.last_used_step.cmp(&left.last_used_step))
            .then_with(|| left.locator.cmp(&right.locator))
    });
    sources
}

pub(crate) fn prioritized_artifact_references(
    references: &[ArtifactReference],
) -> Vec<ArtifactReference> {
    let mut references = references.to_vec();
    references.sort_by(|left, right| {
        artifact_reference_rank(right)
            .cmp(&artifact_reference_rank(left))
            .then_with(|| left.locator.cmp(&right.locator))
    });
    references
}

fn working_source_rank(source: &WorkingSource) -> u8 {
    let role_rank = match source.role.as_str() {
        "authoritative" => 4,
        "generated" => 3,
        "supporting" => 2,
        "candidate" => 1,
        _ => 0,
    };
    let status_rank = match source.status.as_str() {
        "read" | "excerpted" | "ingested" => 4,
        "created" | "written" | "overwritten" | "appended" | "command_changed" => 4,
        "matched_text" => 3,
        "matched" => 2,
        "listed" | "inspected" => 1,
        _ => 0,
    };
    role_rank * 10 + status_rank
}

fn artifact_reference_rank(reference: &ArtifactReference) -> u8 {
    match reference.status.as_str() {
        "read" | "structured_read" | "extracted" => 5,
        "created" | "written" | "overwritten" | "appended" | "command_changed" => 4,
        "searched" => 3,
        "matched" => 2,
        "listed" | "inspected" => 1,
        _ => 0,
    }
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.contains(&value) {
        items.push(value);
    }
}

pub(crate) fn artifact_references_for_result(result: &ActionResult) -> Vec<ArtifactReference> {
    match result {
        ActionResult::Command(command) => vec![ArtifactReference {
            kind: "command".to_string(),
            locator: command.command.clone(),
            status: if command.cancelled {
                "cancelled".to_string()
            } else if command.success {
                "executed".to_string()
            } else {
                "failed".to_string()
            },
        }]
        .into_iter()
        .chain(command.observed_paths.iter().map(|path| ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: "command_changed".to_string(),
        }))
        .collect(),
        ActionResult::Inspection(world) => world
            .files
            .iter()
            .take(6)
            .map(|item| ArtifactReference {
                kind: "path".to_string(),
                locator: item.path.display().to_string(),
                status: "inspected".to_string(),
            })
            .collect(),
        ActionResult::DirectoryListing { root, .. } => vec![ArtifactReference {
            kind: "directory".to_string(),
            locator: root.display().to_string(),
            status: "listed".to_string(),
        }],
        ActionResult::FileMatches { matches, .. } => matches
            .iter()
            .take(6)
            .map(|path| ArtifactReference {
                kind: if path.is_dir() { "directory" } else { "file" }.to_string(),
                locator: path.display().to_string(),
                status: "matched".to_string(),
            })
            .collect(),
        ActionResult::FileRead { path, .. } => vec![ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: "read".to_string(),
        }],
        ActionResult::StructuredData { path, .. } => vec![ArtifactReference {
            kind: "structured_data".to_string(),
            locator: path.display().to_string(),
            status: "structured_read".to_string(),
        }],
        ActionResult::DocumentText {
            path, page_range, ..
        } => vec![ArtifactReference {
            kind: "document".to_string(),
            locator: document_locator_with_page_range(path, page_range.as_ref()),
            status: "extracted".to_string(),
        }],
        ActionResult::TextSearch { matches, .. } => matches
            .iter()
            .take(6)
            .map(|item| ArtifactReference {
                kind: "file".to_string(),
                locator: item.path.display().to_string(),
                status: "searched".to_string(),
            })
            .collect(),
        ActionResult::McpResources { resources, .. } => resources
            .iter()
            .take(8)
            .map(|resource| ArtifactReference {
                kind: "mcp_resource".to_string(),
                locator: format!("{}/{}", resource.server, resource.uri),
                status: "listed".to_string(),
            })
            .collect(),
        ActionResult::McpResourceRead(result) => vec![ArtifactReference {
            kind: "mcp_resource".to_string(),
            locator: format!("{}/{}", result.server, result.uri),
            status: "read".to_string(),
        }],
        ActionResult::McpToolCall(result) => vec![ArtifactReference {
            kind: "mcp_tool".to_string(),
            locator: format!("{}/{}", result.server, result.tool),
            status: if result.is_error {
                "failed".to_string()
            } else {
                "executed".to_string()
            },
        }],
        ActionResult::FileWrite {
            path,
            created,
            overwritten,
            appended,
            ..
        } => vec![ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: if *appended {
                "appended".to_string()
            } else if *overwritten {
                "overwritten".to_string()
            } else if *created {
                "created".to_string()
            } else {
                "written".to_string()
            },
        }],
        ActionResult::DelegatedTask(result) => {
            let mut refs = vec![ArtifactReference {
                kind: "task".to_string(),
                locator: result.task_id.to_string(),
                status: format!("delegated_{:?}", result.status).to_lowercase(),
            }];
            if let Some(path) = &result.output_path {
                refs.push(ArtifactReference {
                    kind: "file".to_string(),
                    locator: path.display().to_string(),
                    status: "child_output".to_string(),
                });
            }
            refs
        }
        ActionResult::NoteRecorded { .. } | ActionResult::Response { .. } => Vec::new(),
    }
}

pub(crate) fn working_sources_for_result(
    action: &Action,
    result: &ActionResult,
    step_index: usize,
) -> Vec<WorkingSource> {
    match result {
        ActionResult::Inspection(world) => world
            .files
            .iter()
            .take(6)
            .map(|item| WorkingSource {
                kind: "path".to_string(),
                locator: item.path.display().to_string(),
                role: "supporting".to_string(),
                status: "inspected".to_string(),
                why_it_matters: format!("observed while {}", action_label(action)),
                last_used_step: step_index,
                evidence_refs: vec![item.path.display().to_string()],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: None,
            })
            .collect(),
        ActionResult::DirectoryListing { root, .. } => vec![WorkingSource {
            kind: "directory".to_string(),
            locator: root.display().to_string(),
            role: "supporting".to_string(),
            status: "listed".to_string(),
            why_it_matters: "directory explored for task-relevant candidates".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![root.display().to_string()],
            page_reference: None,
            extraction_method: None,
            structured_summary: None,
            preview_excerpt: None,
        }],
        ActionResult::FileMatches { matches, .. } => matches
            .iter()
            .take(6)
            .map(|path| WorkingSource {
                kind: if path.is_dir() { "directory" } else { "file" }.to_string(),
                locator: path.display().to_string(),
                role: "candidate".to_string(),
                status: "matched".to_string(),
                why_it_matters: "candidate source discovered for the task".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![path.display().to_string()],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: None,
            })
            .collect(),
        ActionResult::FileRead { path, content, .. } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "content source currently informing the task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
            page_reference: None,
            extraction_method: Some("text_read".to_string()),
            structured_summary: None,
            preview_excerpt: Some(preview_text(content, 100)),
        }],
        ActionResult::StructuredData {
            path,
            headers,
            rows,
            total_rows,
            extraction_method,
            ..
        } => vec![WorkingSource {
            kind: "structured_data".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "ingested".to_string(),
            why_it_matters: "structured local data currently informing the task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
            page_reference: None,
            extraction_method: Some(extraction_method.clone()),
            structured_summary: Some(StructuredSourceSummary {
                headers: headers.clone(),
                sample_rows: rows.len(),
                total_rows: *total_rows,
            }),
            preview_excerpt: None,
        }],
        ActionResult::DocumentText {
            path,
            content,
            extraction_method,
            page_range,
            ..
        } => vec![WorkingSource {
            kind: "document".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "excerpted".to_string(),
            why_it_matters: "document text extracted for task reasoning".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![document_locator_with_page_range(path, page_range.as_ref())],
            page_reference: page_range.as_ref().map(DocumentPageRange::render),
            extraction_method: Some(extraction_method.clone()),
            structured_summary: None,
            preview_excerpt: Some(preview_text(content, 100)),
        }],
        ActionResult::TextSearch { matches, .. } => matches
            .iter()
            .take(6)
            .map(|item| WorkingSource {
                kind: "file".to_string(),
                locator: item.path.display().to_string(),
                role: "supporting".to_string(),
                status: "matched_text".to_string(),
                why_it_matters: "contains text evidence relevant to the task".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![format!("{}:{}", item.path.display(), item.line_number)],
                page_reference: None,
                extraction_method: None,
                structured_summary: None,
                preview_excerpt: Some(preview_text(&item.line, 100)),
            })
            .collect(),
        ActionResult::McpResources { resources, .. } => resources
            .iter()
            .take(8)
            .map(|resource| WorkingSource {
                kind: "mcp_resource".to_string(),
                locator: format!("{}/{}", resource.server, resource.uri),
                role: "candidate".to_string(),
                status: "listed".to_string(),
                why_it_matters: "MCP resource discovered as potential task evidence".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![format!("{}/{}", resource.server, resource.uri)],
                page_reference: None,
                extraction_method: Some("mcp_list_resources".to_string()),
                structured_summary: None,
                preview_excerpt: resource.description.clone(),
            })
            .collect(),
        ActionResult::McpResourceRead(result) => vec![WorkingSource {
            kind: "mcp_resource".to_string(),
            locator: format!("mcp-resource://{}/{}", result.server, result.uri),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "MCP resource content is informing the current task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![format!("mcp-resource://{}/{}", result.server, result.uri)],
            page_reference: None,
            extraction_method: Some("mcp_read_resource".to_string()),
            structured_summary: None,
            preview_excerpt: result
                .contents
                .iter()
                .find_map(|content| content.text.clone())
                .map(|text| preview_text(&text, 100)),
        }],
        ActionResult::McpToolCall(result) => vec![WorkingSource {
            kind: "mcp_tool".to_string(),
            locator: format!("mcp-tool://{}/{}", result.server, result.tool),
            role: if result.is_error {
                "supporting".to_string()
            } else {
                "authoritative".to_string()
            },
            status: if result.is_error {
                "failed".to_string()
            } else {
                "executed".to_string()
            },
            why_it_matters: "MCP tool output contributed external task evidence".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![format!("mcp-tool://{}/{}", result.server, result.tool)],
            page_reference: None,
            extraction_method: Some("mcp_call".to_string()),
            structured_summary: None,
            preview_excerpt: Some(mcp_tool_preview_excerpt(result)),
        }],
        ActionResult::FileWrite {
            path,
            created,
            overwritten,
            appended,
            preview_excerpt,
            artifact,
            ..
        } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "generated".to_string(),
            status: if *appended {
                "appended".to_string()
            } else if *overwritten {
                "overwritten".to_string()
            } else if *created {
                "created".to_string()
            } else {
                "written".to_string()
            },
            why_it_matters: "task produced or updated this artifact".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
            page_reference: None,
            extraction_method: Some("file_write".to_string()),
            structured_summary: None,
            preview_excerpt: preview_excerpt
                .clone()
                .or_else(|| Some(preview_text(&artifact.final_content, 100))),
        }],
        ActionResult::Command(command) => {
            let mut sources = vec![WorkingSource {
                kind: "command".to_string(),
                locator: command.command.clone(),
                role: "supporting".to_string(),
                status: if command.cancelled {
                    "cancelled".to_string()
                } else if command.success {
                    "executed".to_string()
                } else {
                    "failed".to_string()
                },
                why_it_matters: "shell command executed as part of the task".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![command.command.clone()],
                page_reference: None,
                extraction_method: Some("run_command".to_string()),
                structured_summary: None,
                preview_excerpt: Some(preview_text(&command.stdout, 100)),
            }];
            sources.extend(command.observed_paths.iter().map(|path| WorkingSource {
                kind: "file".to_string(),
                locator: path.display().to_string(),
                role: "generated".to_string(),
                status: "command_changed".to_string(),
                why_it_matters: "shell command created or modified this artifact".to_string(),
                last_used_step: step_index,
                evidence_refs: vec![path.display().to_string()],
                page_reference: None,
                extraction_method: Some("run_command".to_string()),
                structured_summary: None,
                preview_excerpt: None,
            }));
            sources
        }
        ActionResult::DelegatedTask(result) => vec![WorkingSource {
            kind: "local_agent".to_string(),
            locator: result.task_id.to_string(),
            role: "supporting".to_string(),
            status: format!("{:?}", result.status).to_lowercase(),
            why_it_matters: "delegated child worker completed bounded subtask work".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![result.task_id.to_string()],
            page_reference: None,
            extraction_method: Some("agent_spawn".to_string()),
            structured_summary: None,
            preview_excerpt: Some(preview_text(
                result
                    .transcript_excerpt
                    .as_deref()
                    .unwrap_or(&result.summary),
                160,
            )),
        }],
        ActionResult::NoteRecorded { note } => vec![WorkingSource {
            kind: "note".to_string(),
            locator: preview_text(note, 80),
            role: "generated".to_string(),
            status: "recorded".to_string(),
            why_it_matters: "operator/task note captured by the harness".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![preview_text(note, 80)],
            page_reference: None,
            extraction_method: Some("record_note".to_string()),
            structured_summary: None,
            preview_excerpt: Some(preview_text(note, 80)),
        }],
        ActionResult::Response { .. } => Vec::new(),
    }
}

fn preview_paths(paths: Vec<PathBuf>) -> String {
    let preview = paths
        .into_iter()
        .take(3)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "no examples".to_string();
    }
    preview.join(", ")
}

fn normalize_command_family(command: &str) -> String {
    let normalized = normalize_shellish_command(command);
    let subject = command_subject_key(command);

    if normalized.contains("ps aux") && normalized.contains("grep ") {
        if let Some(target) = subject.clone().or_else(|| extract_grep_target(&normalized)) {
            return format!("process_check:{target}");
        }
        return "process_check".to_string();
    }

    if normalized.starts_with("pgrep ") {
        if let Some(target) = subject
            .clone()
            .or_else(|| extract_last_non_flag_token(&normalized))
        {
            return format!("process_check:{target}");
        }
        return "process_check".to_string();
    }

    if normalized.contains("pkill ") || normalized.starts_with("killall ") {
        if let Some(target) = subject
            .clone()
            .or_else(|| extract_last_non_flag_token(&normalized))
        {
            return format!("process_kill:{target}");
        }
        return "process_kill".to_string();
    }

    if normalized.contains("osascript") && normalized.contains("quit app") {
        if let Some(target) = subject
            .or_else(|| extract_quoted_app_name(command).map(|name| name.to_ascii_lowercase()))
        {
            return format!("app_quit:{}", target.to_ascii_lowercase());
        }
        return "app_quit".to_string();
    }

    normalized
}

fn command_subject_key(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("docker") {
        return Some("docker".to_string());
    }
    if lower.contains("github") {
        return Some("github".to_string());
    }
    None
}

fn normalize_shellish_command(command: &str) -> String {
    let mut normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");

    if let Some((base, _)) = normalized.split_once(" || echo ") {
        normalized = base.trim().to_string();
    }
    if let Some((base, _)) = normalized.split_once(" | head ") {
        normalized = base.trim().to_string();
    }
    if let Some((base, _)) = normalized.split_once(" | tail ") {
        normalized = base.trim().to_string();
    }

    normalized
}

fn extract_grep_target(command: &str) -> Option<String> {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        if *token != "grep" {
            continue;
        }
        let mut cursor = index + 1;
        while cursor < tokens.len() {
            let candidate = tokens[cursor];
            if candidate.starts_with('-') {
                cursor += 1;
                continue;
            }
            if candidate == "grep" || candidate == "|" {
                break;
            }
            return Some(
                candidate
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_ascii_lowercase(),
            );
        }
    }
    None
}

fn extract_last_non_flag_token(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .rev()
        .find(|token| !token.starts_with('-') && *token != "|" && *token != "&&")
        .map(|token| {
            token
                .trim_matches(|c| c == '"' || c == '\'')
                .to_ascii_lowercase()
        })
}

fn extract_quoted_app_name(command: &str) -> Option<String> {
    let (_, suffix) = command.split_once("quit app ")?;
    let quote = suffix.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let closing = suffix[1..].find(quote)?;
    Some(suffix[1..1 + closing].to_string())
}

fn preview_search_matches(matches: &[SearchMatch]) -> String {
    let preview = matches
        .iter()
        .take(3)
        .map(|item| {
            format!(
                "{}:{} {}",
                item.path.display(),
                item.line_number,
                preview_text(&item.line, 60)
            )
        })
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "no examples".to_string();
    }
    preview.join(" | ")
}

fn preview_structured_rows(rows: &[StructuredDataRow], limit: usize) -> String {
    let preview = rows
        .iter()
        .take(limit)
        .map(|row| format!("row {}: {}", row.row_number, row.values.join(" | ")))
        .collect::<Vec<_>>();
    if preview.is_empty() {
        return "no sample rows".to_string();
    }
    preview.join("; ")
}

fn summarize_mcp_tool_signal(result: &McpToolCallResult) -> String {
    if result.is_error {
        return format!(": {}", preview_text(&result.content_preview, 120));
    }

    let highlights = mcp_result_highlights(result);
    if highlights.is_empty() {
        String::new()
    } else {
        format!(" [{}]", highlights.join(" | "))
    }
}

fn mcp_tool_preview_excerpt(result: &McpToolCallResult) -> String {
    let highlights = mcp_result_highlights(result);
    if highlights.is_empty() {
        preview_text(&result.content_preview, 100)
    } else {
        preview_text(&highlights.join(" | "), 160)
    }
}

fn mcp_result_highlights(result: &McpToolCallResult) -> Vec<String> {
    let Some(structured) = &result.structured_content else {
        return Vec::new();
    };
    collect_json_highlights(structured, 3)
}

fn collect_json_highlights(value: &serde_json::Value, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    collect_json_highlights_into(value, limit, &mut out);
    out
}

fn collect_json_highlights_into(value: &serde_json::Value, limit: usize, out: &mut Vec<String>) {
    if out.len() >= limit {
        return;
    }

    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                if out.len() >= limit {
                    break;
                }
                collect_json_highlights_into(item, limit, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(summary) = summarize_json_object_hit(map) {
                out.push(summary);
                if out.len() >= limit {
                    return;
                }
            }
            for child in map.values() {
                if out.len() >= limit {
                    break;
                }
                collect_json_highlights_into(child, limit, out);
            }
        }
        _ => {}
    }
}

fn summarize_json_object_hit(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let title = first_string(map, &["title", "name", "headline"])?;
    let mut summary = preview_text(&title, 80);
    if let Some(url) = first_string(map, &["url", "link"]) {
        summary.push_str(" -> ");
        summary.push_str(&preview_text(&url, 100));
    } else if let Some(description) = first_string(map, &["description", "snippet", "summary"]) {
        summary.push_str(": ");
        summary.push_str(&preview_text(&description, 80));
    }
    Some(summary)
}

fn first_string(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn document_locator_with_page_range(path: &Path, page_range: Option<&DocumentPageRange>) -> String {
    page_range
        .map(|range| format!("{} ({})", path.display(), range.render()))
        .unwrap_or_else(|| path.display().to_string())
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}
