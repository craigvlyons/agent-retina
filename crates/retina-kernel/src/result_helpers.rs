use super::action_label;
use retina_types::*;
use std::path::{Path, PathBuf};

pub(crate) fn summarize_action_result(result: &ActionResult) -> String {
    match result {
        ActionResult::Command(command) => format!(
            "command {} with exit {:?}",
            if command.success {
                "succeeded"
            } else {
                "failed"
            },
            command.exit_code
        ),
        ActionResult::Inspection(world) => format!("inspected {} path(s)", world.files.len()),
        ActionResult::DirectoryListing { root, entries } => format!(
            "listed {} entr{} under {} [{}]",
            entries.len(),
            if entries.len() == 1 { "y" } else { "ies" },
            root.display(),
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
        ActionResult::FileWrite {
            path,
            bytes_written,
            appended,
        } => format!(
            "{} {} ({} bytes)",
            if *appended { "appended to" } else { "wrote" },
            path.display(),
            bytes_written
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
        ActionResult::DirectoryListing { root, entries } => serde_json::json!({
            "type": "directory_listing",
            "root": root,
            "count": entries.len(),
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
        ActionResult::FileWrite {
            path,
            bytes_written,
            appended,
        } => serde_json::json!({
            "type": "file_write",
            "path": path,
            "bytes_written": bytes_written,
            "appended": appended,
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

pub(crate) fn summarize_verified_facts(references: &[ArtifactReference]) -> Vec<String> {
    references
        .iter()
        .take(8)
        .map(|reference| match reference.status.as_str() {
            "read" => format!("read {}", reference.locator),
            "extracted" => format!("extracted text from {}", reference.locator),
            "matched" => format!("matched candidate {}", reference.locator),
            "searched" => format!("searched and found evidence in {}", reference.locator),
            "listed" => format!("listed directory {}", reference.locator),
            "written" => format!("wrote {}", reference.locator),
            "appended" => format!("appended {}", reference.locator),
            _ => format!("{} {}", reference.status, reference.locator),
        })
        .collect()
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
        }],
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
        ActionResult::FileWrite { path, appended, .. } => vec![ArtifactReference {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            status: if *appended {
                "appended".to_string()
            } else {
                "written".to_string()
            },
        }],
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
            })
            .collect(),
        ActionResult::FileRead { path, .. } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "authoritative".to_string(),
            status: "read".to_string(),
            why_it_matters: "content source currently informing the task".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
            page_reference: None,
            extraction_method: Some("text_read".to_string()),
        }],
        ActionResult::DocumentText {
            path,
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
            })
            .collect(),
        ActionResult::FileWrite { path, appended, .. } => vec![WorkingSource {
            kind: "file".to_string(),
            locator: path.display().to_string(),
            role: "generated".to_string(),
            status: if *appended {
                "appended".to_string()
            } else {
                "written".to_string()
            },
            why_it_matters: "task produced or updated this artifact".to_string(),
            last_used_step: step_index,
            evidence_refs: vec![path.display().to_string()],
            page_reference: None,
            extraction_method: Some("file_write".to_string()),
        }],
        ActionResult::Command(command) => vec![WorkingSource {
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
        }],
        ActionResult::Response { .. } => Vec::new(),
    }
}

pub(crate) fn should_retry(previous: &Action, next: &Action) -> bool {
    action_label(previous) != action_label(next) && !matches!(next, Action::Respond { .. })
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
