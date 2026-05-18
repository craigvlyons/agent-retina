// File boundary: keep lib.rs focused on shell wiring and the top-level CLI shell
// type. Move file/process/state helpers into sibling modules before growth.
mod file_edit;
mod file_ops;
mod notebook_edit;
mod policy;
mod process_control;
mod read_state;
mod state_helpers;
mod text_write;

pub use policy::ScopedShell;

use crate::state_helpers::{command_fingerprint, path_fingerprint};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use pdf_extract::{extract_text, extract_text_by_pages};
use retina_traits::Shell;
use retina_types::*;
use std::collections::HashMap;
use std::fs::{self};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, MutexGuard};

const DEFAULT_MAX_READ_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_LIST_ENTRIES: usize = 200;
const DEFAULT_MAX_SEARCH_RESULTS: usize = 50;

pub struct CliShell {
    last_command: Mutex<Option<CommandResult>>,
    notes: Mutex<Vec<String>>,
    read_states: Mutex<HashMap<PathBuf, read_state::StoredReadState>>,
}

fn lock_state<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| KernelError::Execution("cli shell state mutex poisoned".to_string()))
}

impl Default for CliShell {
    fn default() -> Self {
        Self::new()
    }
}

impl CliShell {
    pub fn new() -> Self {
        Self {
            last_command: Mutex::new(None),
            notes: Mutex::new(Vec::new()),
            read_states: Mutex::new(HashMap::new()),
        }
    }
}

impl Shell for CliShell {
    fn observe(&self) -> Result<WorldState> {
        Ok(WorldState {
            cwd: std::env::current_dir()?,
            files: Vec::new(),
            last_command: lock_state(&self.last_command)?.clone(),
            notes: lock_state(&self.notes)?.clone(),
        })
    }

    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
        let cwd = std::env::current_dir()?;
        let files = scope
            .tracked_paths
            .iter()
            .map(|tracked| {
                let resolved = Self::resolve_path(&tracked.path)?;
                Self::inspect_path_state(&resolved, tracked.include_content)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(StateSnapshot {
            scope: scope.clone(),
            cwd: cwd.clone(),
            cwd_hash: if scope.include_working_directory {
                Self::cwd_hash(&cwd)
            } else {
                String::new()
            },
            files,
            last_command: if scope.include_last_command {
                lock_state(&self.last_command)?.clone()
            } else {
                None
            },
        })
    }

    fn compare_state(
        &self,
        before: &StateSnapshot,
        after: &StateSnapshot,
        action: Option<&Action>,
    ) -> Result<StateDelta> {
        let mut changed_paths = Vec::new();
        for after_path in &after.files {
            let before_path = before
                .files
                .iter()
                .find(|candidate| candidate.path == after_path.path);
            if before_path.map(path_fingerprint) != Some(path_fingerprint(after_path)) {
                changed_paths.push(after_path.path.clone());
            }
        }

        let cwd_changed = before.cwd_hash != after.cwd_hash && !before.cwd_hash.is_empty();
        let command_changed = before.last_command.as_ref().map(command_fingerprint)
            != after.last_command.as_ref().map(command_fingerprint);
        let changed = !changed_paths.is_empty() || cwd_changed || command_changed;
        let expects_change = action.map(Action::expects_change).unwrap_or(false);

        let kind = match (changed, expects_change) {
            (true, true) => StateDeltaKind::ChangedAsExpected,
            (true, false) => StateDeltaKind::ChangedUnexpectedly,
            (false, true) => StateDeltaKind::Unchanged,
            (false, false) => StateDeltaKind::Unchanged,
        };

        let summary = if changed_paths.is_empty() {
            if changed {
                "state changed outside tracked files".to_string()
            } else {
                "no state change detected".to_string()
            }
        } else {
            format!(
                "changed path(s): {}",
                changed_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        Ok(StateDelta {
            kind,
            summary,
            changed_paths,
        })
    }

    fn execute(&self, action: &Action) -> Result<ActionResult> {
        self.execute_controlled(action, None)
    }

    fn execute_controlled(
        &self,
        action: &Action,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<ActionResult> {
        match action {
            Action::RunCommand {
                command,
                cwd,
                require_approval,
                ..
            } => {
                if classify_privileged_command(command).is_some() && !require_approval {
                    return Err(KernelError::ApprovalDenied(
                        "delete-like or kill-like command blocked until explicitly approved"
                            .to_string(),
                    ));
                }

                let result = Self::run_command(command, cwd.clone(), control)?;
                *lock_state(&self.last_command)? = Some(result.clone());
                Ok(ActionResult::Command(result))
            }
            Action::InspectPath {
                path,
                include_content,
                ..
            } => Ok(ActionResult::Inspection(WorldState {
                cwd: std::env::current_dir()?,
                files: vec![Self::inspect_path_state(
                    &Self::resolve_path(path)?,
                    *include_content,
                )?],
                last_command: lock_state(&self.last_command)?.clone(),
                notes: lock_state(&self.notes)?.clone(),
            })),
            Action::InspectWorkingDirectory { .. } => self.observe().map(ActionResult::Inspection),
            Action::ListDirectory {
                path,
                recursive,
                max_entries,
                ..
            } => {
                let root = Self::resolve_path(path)?;
                let entries = Self::list_directory(
                    path,
                    *recursive,
                    (*max_entries).min(DEFAULT_MAX_LIST_ENTRIES),
                )?;
                let summary = Self::summarize_directory_entries(&entries);
                Ok(ActionResult::DirectoryListing {
                    root,
                    entries,
                    summary,
                })
            }
            Action::FindFiles {
                root,
                pattern,
                recursive,
                max_results,
                offset,
                ..
            } => {
                let result = Self::find_files(
                    root,
                    pattern,
                    *recursive,
                    (*max_results).min(DEFAULT_MAX_SEARCH_RESULTS),
                    *offset,
                )?;
                Ok(ActionResult::FileMatches {
                    root: Self::resolve_path(root)?,
                    pattern: pattern.clone(),
                    matches: result.matches,
                    truncated: result.truncated,
                    applied_offset: result.applied_offset,
                })
            }
            Action::SearchText {
                root,
                query,
                max_results,
                offset,
                glob,
                case_insensitive,
                output_mode,
                ..
            } => {
                let result = Self::search_text(
                    root,
                    query,
                    (*max_results).min(DEFAULT_MAX_SEARCH_RESULTS),
                    *offset,
                    glob.as_deref(),
                    *case_insensitive,
                    output_mode,
                )?;
                Ok(ActionResult::TextSearch {
                    root: Self::resolve_path(root)?,
                    query: query.clone(),
                    output_mode: output_mode.clone(),
                    matches: result.matches,
                    content: result.content,
                    filenames: result.filenames,
                    num_files: result.num_files,
                    num_matches: result.num_matches,
                    truncated: result.truncated,
                    applied_offset: result.applied_offset,
                    glob: glob.clone(),
                    case_insensitive: *case_insensitive,
                })
            }
            Action::ReadFile {
                path,
                start_line,
                limit_lines,
                max_bytes,
                ..
            } => {
                let read = Self::read_file(path, *start_line, *limit_lines, *max_bytes)?;
                let resolved = Self::resolve_path(path)?;
                self.maybe_remember_text_read(&resolved, &read.content, read.was_partial)?;
                Ok(ActionResult::FileRead {
                    path: resolved,
                    content: read.content,
                    truncated: read.truncated,
                    start_line: read.start_line,
                    line_count: read.line_count,
                    total_lines: read.total_lines,
                    total_bytes: read.total_bytes,
                    read_bytes: read.read_bytes,
                })
            }
            Action::IngestStructuredData { path, max_rows, .. } => {
                let structured = Self::ingest_structured_data(path, *max_rows)?;
                Ok(ActionResult::StructuredData {
                    path: Self::resolve_path(path)?,
                    format: structured.format,
                    headers: structured.headers,
                    rows: structured.rows,
                    total_rows: structured.total_rows,
                    truncated: structured.truncated,
                    extraction_method: structured.extraction_method,
                })
            }
            Action::ExtractDocumentText {
                path,
                max_chars,
                page_start,
                page_end,
                ..
            } => {
                let (
                    content,
                    truncated,
                    format,
                    extraction_method,
                    page_range,
                    structured_rows_detected,
                ) = Self::extract_document_text(path, *max_chars, *page_start, *page_end)?;
                Ok(ActionResult::DocumentText {
                    path: Self::resolve_path(path)?,
                    content,
                    truncated,
                    format,
                    extraction_method,
                    page_range,
                    structured_rows_detected,
                })
            }
            Action::WriteFile {
                path,
                content,
                overwrite,
                ..
            } => {
                let result = self.write_file(path, content, *overwrite)?;
                Ok(ActionResult::FileWrite {
                    path: result.path,
                    mutation_kind: result.mutation_kind,
                    bytes_written: result.bytes_written,
                    created: result.created,
                    overwritten: result.overwritten,
                    appended: result.appended,
                    original_hash: result.original_hash,
                    updated_hash: result.updated_hash,
                    changed_line_count: result.changed_line_count,
                    patch_summary: result.patch_summary,
                    preview_excerpt: result.preview_excerpt,
                    artifact: result.artifact,
                })
            }
            Action::EditFile {
                path,
                old_string,
                new_string,
                replace_all,
                ..
            } => {
                let result = self.edit_file(path, old_string, new_string, *replace_all)?;
                Ok(ActionResult::FileWrite {
                    path: result.path,
                    mutation_kind: result.mutation_kind,
                    bytes_written: result.bytes_written,
                    created: result.created,
                    overwritten: result.overwritten,
                    appended: result.appended,
                    original_hash: result.original_hash,
                    updated_hash: result.updated_hash,
                    changed_line_count: result.changed_line_count,
                    patch_summary: result.patch_summary,
                    preview_excerpt: result.preview_excerpt,
                    artifact: result.artifact,
                })
            }
            Action::AppendFile { path, content, .. } => {
                let result = self.append_file(path, content)?;
                Ok(ActionResult::FileWrite {
                    path: result.path,
                    mutation_kind: result.mutation_kind,
                    bytes_written: result.bytes_written,
                    created: result.created,
                    overwritten: result.overwritten,
                    appended: result.appended,
                    original_hash: result.original_hash,
                    updated_hash: result.updated_hash,
                    changed_line_count: result.changed_line_count,
                    patch_summary: result.patch_summary,
                    preview_excerpt: result.preview_excerpt,
                    artifact: result.artifact,
                })
            }
            Action::EditNotebook {
                path,
                cell_id,
                new_source,
                cell_type,
                edit_mode,
                ..
            } => {
                let result = self.edit_notebook(
                    path,
                    cell_id.clone(),
                    new_source,
                    cell_type.clone(),
                    edit_mode.clone(),
                )?;
                Ok(ActionResult::FileWrite {
                    path: result.path,
                    mutation_kind: result.mutation_kind,
                    bytes_written: result.bytes_written,
                    created: result.created,
                    overwritten: result.overwritten,
                    appended: result.appended,
                    original_hash: result.original_hash,
                    updated_hash: result.updated_hash,
                    changed_line_count: result.changed_line_count,
                    patch_summary: result.patch_summary,
                    preview_excerpt: result.preview_excerpt,
                    artifact: result.artifact,
                })
            }
            Action::ListMcpResources { .. }
            | Action::ReadMcpResource { .. }
            | Action::CallMcpTool { .. } => Err(KernelError::Unsupported(
                "MCP actions must be dispatched through the MCP runtime".to_string(),
            )),
            Action::SpawnAgent { .. } => Err(KernelError::Unsupported(
                "spawn_agent must be dispatched through the local agent runtime".to_string(),
            )),
            Action::RecordNote { note, .. } => {
                lock_state(&self.notes)?.push(note.clone());
                Ok(ActionResult::NoteRecorded { note: note.clone() })
            }
            Action::Respond { message, .. } => Ok(ActionResult::Response {
                message: message.clone(),
            }),
        }
    }

    fn restore_read_state_cache(&self, states: &[CachedFileReadState]) -> Result<()> {
        self.remember_restored_read_states(states)
    }

    fn constraints(&self) -> &[HardConstraint] {
        static CONSTRAINTS: [HardConstraint; 1] = [HardConstraint::DeleteOrKillRequireApproval];
        &CONSTRAINTS
    }

    fn capabilities(&self) -> ShellCapabilities {
        ShellCapabilities {
            can_execute_commands: true,
            can_read_files: true,
            can_write_files: true,
            can_search_files: true,
            can_extract_documents: true,
            can_write_notes: true,
            can_respond_text: true,
        }
    }

    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse> {
        print!(
            "Approve action '{}' because {}? [y/N]: ",
            request.action, request.reason
        );
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let normalized = input.trim().to_lowercase();
        if normalized == "y" || normalized == "yes" {
            Ok(ApprovalResponse::Approved)
        } else {
            Ok(ApprovalResponse::Denied)
        }
    }

    fn notify(&self, message: &str) -> Result<()> {
        println!("{message}");
        Ok(())
    }

    fn request_input(&self, prompt: &str) -> Result<String> {
        print!("{prompt}: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> T {
        result.unwrap_or_else(|error| panic!("test operation failed: {error}"))
    }

    fn must_tempdir() -> tempfile::TempDir {
        tempdir().unwrap_or_else(|error| panic!("failed to create tempdir: {error}"))
    }

    #[test]
    fn command_execution_captures_output() {
        let shell = CliShell::new();
        #[cfg(unix)]
        let command = "printf 'hello'";
        #[cfg(windows)]
        let command = "[Console]::Out.Write('hello')";
        let result = must(shell.execute(&Action::RunCommand {
            id: ActionId::new(),
            command: command.to_string(),
            cwd: None,
            require_approval: false,
            expect_change: false,
            state_scope: HashScope::default(),
        }));
        let ActionResult::Command(result) = result else {
            panic!("expected command result");
        };
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.duration_ms <= 5_000);
    }

    #[test]
    fn delete_like_command_requires_explicit_approval() {
        let shell = CliShell::new();
        let error = match shell.execute(&Action::RunCommand {
            id: ActionId::new(),
            command: "rm tmp/test.txt".to_string(),
            cwd: None,
            require_approval: false,
            expect_change: true,
            state_scope: HashScope::default(),
        }) {
            Ok(_) => panic!("expected approval-denied error"),
            Err(error) => error,
        };
        assert!(matches!(error, KernelError::ApprovalDenied(_)));
    }

    #[test]
    fn controlled_command_can_be_cancelled() {
        let shell = CliShell::new();
        let control = ExecutionControl::new();
        let handle = control.handle();
        let cancel_handle = handle.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel_handle.request_cancel();
        });
        #[cfg(unix)]
        let command = "sleep 5";
        #[cfg(windows)]
        let command = "Start-Sleep -Seconds 5";

        let result = must(shell.execute_controlled(
            &Action::RunCommand {
                id: ActionId::new(),
                command: command.to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            },
            Some(&handle),
        ));
        let ActionResult::Command(result) = result else {
            panic!("expected command result");
        };
        assert!(result.cancelled);
        assert!(!result.success);
        assert!(result.termination.is_some());
    }

    #[test]
    fn read_file_returns_content() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "hello retina"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let ActionResult::FileRead {
            path,
            content,
            truncated,
            start_line,
            line_count,
            total_lines,
            ..
        } = result
        else {
            panic!("expected file read");
        };
        assert_eq!(path, file);
        assert_eq!(content, "hello retina");
        assert!(!truncated);
        assert_eq!(start_line, 1);
        assert_eq!(line_count, 1);
        assert_eq!(total_lines, 1);
    }

    #[test]
    fn read_file_supports_targeted_line_ranges() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "alpha\nbeta\ngamma\ndelta\n"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file,
            start_line: Some(2),
            limit_lines: Some(2),
            max_bytes: None,
        }));
        let ActionResult::FileRead {
            content,
            truncated,
            start_line,
            line_count,
            total_lines,
            ..
        } = result
        else {
            panic!("expected file read");
        };
        assert_eq!(content, "beta\ngamma\n");
        assert!(truncated);
        assert_eq!(start_line, 2);
        assert_eq!(line_count, 2);
        assert_eq!(total_lines, 4);
    }

    #[test]
    fn partial_read_does_not_unlock_mutation() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "alpha\nbeta\ngamma\n"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: Some(2),
            limit_lines: Some(1),
            max_bytes: None,
        }));
        let error = shell
            .execute(&Action::EditFile {
                id: ActionId::new(),
                path: file,
                old_string: "beta".to_string(),
                new_string: "delta".to_string(),
                replace_all: false,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("only partially read"));
    }

    #[test]
    fn restored_full_read_state_allows_edit_without_reread() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "alpha\nbeta\n"));
        let shell = CliShell::new();
        let state = must(CliShell::canonical_read_state_from_result(
            &file,
            "alpha\nbeta\n",
            1,
            2,
            2,
            11,
            11,
            false,
        ));
        must(shell.restore_read_state_cache(&[state]));
        let result = must(shell.execute(&Action::EditFile {
            id: ActionId::new(),
            path: file.clone(),
            old_string: "beta".to_string(),
            new_string: "delta".to_string(),
            replace_all: false,
        }));
        let ActionResult::FileWrite { artifact, .. } = result else {
            panic!("expected file write");
        };
        assert_eq!(artifact.final_content, "alpha\ndelta\n");
    }

    #[test]
    fn restored_partial_read_state_still_blocks_edit() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "alpha\nbeta\ngamma\n"));
        let shell = CliShell::new();
        let state = must(CliShell::canonical_read_state_from_result(
            &file, "beta\n", 2, 1, 3, 17, 5, true,
        ));
        must(shell.restore_read_state_cache(&[state]));
        let error = shell
            .execute(&Action::EditFile {
                id: ActionId::new(),
                path: file,
                old_string: "beta".to_string(),
                new_string: "delta".to_string(),
                replace_all: false,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("only partially read"));
    }

    #[test]
    fn overwrite_existing_file_requires_prior_read() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "before"));
        let shell = CliShell::new();
        let error = shell
            .execute(&Action::WriteFile {
                id: ActionId::new(),
                path: file.clone(),
                content: "after".to_string(),
                overwrite: true,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("must be read before it can be modified"));
    }

    #[test]
    fn stale_overwrite_is_rejected_after_file_changes() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "before"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        must(fs::write(&file, "changed elsewhere"));
        let error = shell
            .execute(&Action::WriteFile {
                id: ActionId::new(),
                path: file.clone(),
                content: "after".to_string(),
                overwrite: true,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("changed after it was read"));
    }

    #[test]
    fn edit_file_replaces_exact_match_after_read() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "alpha\nbeta\n"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let result = must(shell.execute(&Action::EditFile {
            id: ActionId::new(),
            path: file.clone(),
            old_string: "beta".to_string(),
            new_string: "gamma".to_string(),
            replace_all: false,
        }));
        let ActionResult::FileWrite {
            mutation_kind,
            patch_summary,
            ..
        } = result
        else {
            panic!("expected file write result");
        };
        assert_eq!(mutation_kind, FileMutationKind::ExactEdit);
        assert_eq!(must(fs::read_to_string(&file)), "alpha\ngamma\n");
        let summary = patch_summary.expect("expected patch summary");
        assert_eq!(summary.replaced_occurrences, 1);
    }

    #[test]
    fn edit_file_rejects_ambiguous_match_without_replace_all() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "beta\nbeta\n"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let error = shell
            .execute(&Action::EditFile {
                id: ActionId::new(),
                path: file.clone(),
                old_string: "beta".to_string(),
                new_string: "gamma".to_string(),
                replace_all: false,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("set replace_all=true"));
    }

    #[test]
    fn edit_file_rejects_noop_edit() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "beta\n"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let error = shell
            .execute(&Action::EditFile {
                id: ActionId::new(),
                path: file,
                old_string: "beta".to_string(),
                new_string: "beta".to_string(),
                replace_all: false,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("nothing to change"));
    }

    #[test]
    fn edit_file_matches_quote_normalized_text() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "say “hello”\n"));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let result = must(shell.execute(&Action::EditFile {
            id: ActionId::new(),
            path: file.clone(),
            old_string: "say \"hello\"".to_string(),
            new_string: "say goodbye".to_string(),
            replace_all: false,
        }));
        let ActionResult::FileWrite { artifact, .. } = result else {
            panic!("expected file write result");
        };
        assert_eq!(must(fs::read_to_string(&file)), "say goodbye\n");
        assert_eq!(artifact.final_content, "say goodbye\n");
    }

    #[test]
    fn text_mutation_rejects_notebooks() {
        let dir = must_tempdir();
        let file = dir.path().join("note.ipynb");
        must(fs::write(&file, notebook_fixture()));
        let shell = CliShell::new();
        let error = shell
            .execute(&Action::WriteFile {
                id: ActionId::new(),
                path: file,
                content: "{}".to_string(),
                overwrite: true,
            })
            .unwrap_err();
        let KernelError::Unsupported(message) = error else {
            panic!("expected unsupported error");
        };
        assert!(message.contains("edit_notebook"));
    }

    #[test]
    fn edit_notebook_rewrites_selected_cell() {
        let dir = must_tempdir();
        let file = dir.path().join("note.ipynb");
        must(fs::write(&file, notebook_fixture()));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let result = must(shell.execute(&Action::EditNotebook {
            id: ActionId::new(),
            path: file.clone(),
            cell_id: Some("intro".to_string()),
            new_source: "# Updated\n".to_string(),
            cell_type: Some(NotebookCellType::Markdown),
            edit_mode: NotebookEditMode::Replace,
        }));
        let ActionResult::FileWrite { mutation_kind, .. } = result else {
            panic!("expected file write");
        };
        assert_eq!(mutation_kind, FileMutationKind::NotebookReplace);
        let updated = must(fs::read_to_string(&file));
        assert!(updated.contains("# Updated"));
    }

    #[test]
    fn edit_notebook_insert_requires_cell_type() {
        let dir = must_tempdir();
        let file = dir.path().join("note.ipynb");
        must(fs::write(&file, notebook_fixture()));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let error = shell
            .execute(&Action::EditNotebook {
                id: ActionId::new(),
                path: file,
                cell_id: Some("intro".to_string()),
                new_source: "print('hi')\n".to_string(),
                cell_type: None,
                edit_mode: NotebookEditMode::Insert,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("requires cell_type"));
    }

    #[test]
    fn edit_notebook_replace_requires_cell_id() {
        let dir = must_tempdir();
        let file = dir.path().join("note.ipynb");
        must(fs::write(&file, notebook_fixture()));
        let shell = CliShell::new();
        must(shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file,
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }));
        let error = shell
            .execute(&Action::EditNotebook {
                id: ActionId::new(),
                path: dir.path().join("note.ipynb"),
                cell_id: None,
                new_source: "# Updated\n".to_string(),
                cell_type: Some(NotebookCellType::Markdown),
                edit_mode: NotebookEditMode::Replace,
            })
            .unwrap_err();
        let KernelError::Validation(message) = error else {
            panic!("expected validation error");
        };
        assert!(message.contains("requires cell_id"));
    }

    #[test]
    fn inspect_path_resolves_known_folder_aliases() {
        let Some(desktop) = dirs::desktop_dir() else {
            return;
        };

        let file = desktop.join("retina-inspect-alias-test.txt");
        let _ = fs::remove_file(&file);
        must(fs::write(&file, "alias inspect content"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::InspectPath {
            id: ActionId::new(),
            path: PathBuf::from("~/Desktop/retina-inspect-alias-test.txt"),
            include_content: true,
        }));
        let ActionResult::Inspection(world) = result else {
            panic!("expected inspection result");
        };
        let inspected = world
            .files
            .first()
            .unwrap_or_else(|| panic!("expected inspected file"));
        assert_eq!(inspected.path, file);
        assert!(inspected.exists);
        assert_eq!(inspected.size, Some("alias inspect content".len() as u64));
        assert!(inspected.content_hash.is_some());

        let _ = fs::remove_file(file);
    }

    #[test]
    fn read_file_rejects_pdf_and_requests_document_tool() {
        let dir = must_tempdir();
        let file = dir.path().join("resume.pdf");
        write_test_pdf_pages(&file, &["hello pdf"]);
        let shell = CliShell::new();
        let error = match shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: file.clone(),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }) {
            Ok(_) => panic!("expected unsupported error"),
            Err(error) => error,
        };
        let KernelError::Unsupported(message) = error else {
            panic!("expected unsupported error");
        };
        assert!(message.contains("extract_document_text"));
    }

    #[test]
    fn read_file_rejects_mcp_tool_locator() {
        let shell = CliShell::new();
        let error = match shell.execute(&Action::ReadFile {
            id: ActionId::new(),
            path: PathBuf::from("mcp-tool://brave/brave_web_search"),
            start_line: None,
            limit_lines: None,
            max_bytes: None,
        }) {
            Ok(_) => panic!("expected unsupported error"),
            Err(error) => error,
        };
        let KernelError::Unsupported(message) = error else {
            panic!("expected unsupported error");
        };
        assert!(message.contains("MCP locator"));
    }

    #[test]
    fn find_files_returns_matches() {
        let dir = must_tempdir();
        let target = dir.path().join("target.txt");
        must(fs::write(&target, "hello"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::FindFiles {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            pattern: "target".to_string(),
            recursive: true,
            max_results: 10,
            offset: 0,
        }));
        let ActionResult::FileMatches { matches, .. } = result else {
            panic!("expected file matches");
        };
        assert_eq!(matches, vec![target]);
    }

    #[test]
    fn find_files_supports_simple_glob_patterns() {
        let dir = must_tempdir();
        let target = dir.path().join("report.txt");
        let other = dir.path().join("report.md");
        must(fs::write(&target, "hello"));
        must(fs::write(&other, "hello"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::FindFiles {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            pattern: "*.txt".to_string(),
            recursive: true,
            max_results: 10,
            offset: 0,
        }));
        let ActionResult::FileMatches { matches, .. } = result else {
            panic!("expected file matches");
        };
        assert_eq!(matches, vec![target]);
    }

    #[test]
    fn find_files_can_stay_top_level_only() {
        let dir = must_tempdir();
        let top_level = dir.path().join("top.txt");
        let nested_dir = dir.path().join("nested");
        let nested = nested_dir.join("deep.txt");
        must(fs::create_dir_all(&nested_dir));
        must(fs::write(&top_level, "hello"));
        must(fs::write(&nested, "hello"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::FindFiles {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            pattern: "*.txt".to_string(),
            recursive: false,
            max_results: 10,
            offset: 0,
        }));
        let ActionResult::FileMatches { matches, .. } = result else {
            panic!("expected file matches");
        };
        assert_eq!(matches, vec![top_level]);
    }

    #[test]
    fn find_files_supports_offset_paging() {
        let dir = must_tempdir();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        let c = dir.path().join("c.txt");
        must(fs::write(&a, "a"));
        must(fs::write(&b, "b"));
        must(fs::write(&c, "c"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::FindFiles {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            pattern: "*.txt".to_string(),
            recursive: true,
            max_results: 1,
            offset: 1,
        }));
        let ActionResult::FileMatches {
            matches,
            truncated,
            applied_offset,
            ..
        } = result
        else {
            panic!("expected file matches");
        };
        assert_eq!(applied_offset, 1);
        assert!(truncated);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], b);
    }

    #[test]
    fn search_text_supports_glob_and_offset() {
        let dir = must_tempdir();
        let rust_file = dir.path().join("main.rs");
        let text_file = dir.path().join("notes.txt");
        must(fs::write(
            &rust_file,
            "Alpha\nbeta\nAlpha again\nALPHA third\n",
        ));
        must(fs::write(&text_file, "Alpha in txt\n"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::SearchText {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            query: "alpha".to_string(),
            max_results: 1,
            offset: 1,
            glob: Some("*.rs".to_string()),
            case_insensitive: true,
            output_mode: TextSearchOutputMode::Content,
        }));
        let ActionResult::TextSearch {
            output_mode,
            matches,
            truncated,
            applied_offset,
            glob,
            case_insensitive,
            ..
        } = result
        else {
            panic!("expected text search");
        };
        assert_eq!(applied_offset, 1);
        assert!(truncated);
        assert_eq!(output_mode, TextSearchOutputMode::Content);
        assert_eq!(glob.as_deref(), Some("*.rs"));
        assert!(case_insensitive);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, rust_file);
        assert_eq!(matches[0].line_number, 3);
    }

    #[test]
    fn search_text_supports_files_with_matches_mode() {
        let dir = must_tempdir();
        let rust_file = dir.path().join("main.rs");
        let docs_file = dir.path().join("guide.md");
        let text_file = dir.path().join("notes.txt");
        must(fs::write(&rust_file, "Alpha\nbeta\n"));
        must(fs::write(&docs_file, "Alpha in docs\n"));
        must(fs::write(&text_file, "Alpha in txt\n"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::SearchText {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            query: "Alpha".to_string(),
            max_results: 1,
            offset: 1,
            glob: None,
            case_insensitive: false,
            output_mode: TextSearchOutputMode::FilesWithMatches,
        }));
        let ActionResult::TextSearch {
            output_mode,
            matches,
            filenames,
            num_files,
            num_matches,
            truncated,
            applied_offset,
            ..
        } = result
        else {
            panic!("expected text search");
        };
        assert_eq!(output_mode, TextSearchOutputMode::FilesWithMatches);
        assert!(matches.is_empty());
        assert_eq!(applied_offset, 1);
        assert!(truncated);
        assert_eq!(filenames.len(), 1);
        assert!(
            filenames[0] == docs_file || filenames[0] == text_file || filenames[0] == rust_file
        );
        assert_eq!(num_files, 1);
        assert_eq!(num_matches, 0);
    }

    #[test]
    fn search_text_supports_count_mode() {
        let dir = must_tempdir();
        let rust_file = dir.path().join("main.rs");
        let text_file = dir.path().join("notes.txt");
        must(fs::write(&rust_file, "Alpha\nbeta\nAlpha again\n"));
        must(fs::write(&text_file, "Alpha in txt\n"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::SearchText {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            query: "Alpha".to_string(),
            max_results: 10,
            offset: 0,
            glob: None,
            case_insensitive: false,
            output_mode: TextSearchOutputMode::Count,
        }));
        let ActionResult::TextSearch {
            output_mode,
            matches,
            content,
            filenames,
            num_files,
            num_matches,
            truncated,
            applied_offset,
            ..
        } = result
        else {
            panic!("expected text search");
        };
        assert_eq!(output_mode, TextSearchOutputMode::Count);
        assert!(matches.is_empty());
        assert!(filenames.is_empty());
        assert!(!truncated);
        assert_eq!(applied_offset, 0);
        assert_eq!(num_files, 2);
        assert_eq!(num_matches, 3);
        let content = content.expect("count content");
        assert!(content.contains("main.rs:2"));
        assert!(content.contains("notes.txt:1"));
    }

    #[test]
    fn state_capture_detects_file_change() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "before"));
        let shell = CliShell::new();
        let scope = HashScope {
            tracked_paths: vec![TrackedPath {
                path: file.clone(),
                include_content: true,
            }],
            include_working_directory: false,
            include_last_command: false,
        };
        let before = must(shell.capture_state(&scope));
        must(fs::write(&file, "after"));
        let after = must(shell.capture_state(&scope));
        let delta = must(shell.compare_state(
            &before,
            &after,
            Some(&Action::WriteFile {
                id: ActionId::new(),
                path: file.clone(),
                content: "after".to_string(),
                overwrite: true,
            }),
        ));
        assert!(matches!(delta.kind, StateDeltaKind::ChangedAsExpected));
        assert_eq!(delta.changed_paths, vec![file]);
    }

    #[test]
    fn state_capture_reports_unchanged_for_noop() {
        let dir = must_tempdir();
        let file = dir.path().join("note.txt");
        must(fs::write(&file, "same"));
        let shell = CliShell::new();
        let scope = HashScope {
            tracked_paths: vec![TrackedPath {
                path: file,
                include_content: true,
            }],
            include_working_directory: false,
            include_last_command: false,
        };
        let before = must(shell.capture_state(&scope));
        let after = must(shell.capture_state(&scope));
        let delta = must(shell.compare_state(&before, &after, None));
        assert!(matches!(delta.kind, StateDeltaKind::Unchanged));
    }

    #[test]
    fn state_capture_resolves_known_folder_aliases_for_write_targets() {
        let Some(desktop) = dirs::desktop_dir() else {
            return;
        };

        let file = desktop.join("retina-state-capture-alias-test.txt");
        let _ = fs::remove_file(&file);
        let shell = CliShell::new();
        let scope = HashScope {
            tracked_paths: vec![TrackedPath {
                path: PathBuf::from("~/Desktop/retina-state-capture-alias-test.txt"),
                include_content: true,
            }],
            include_working_directory: false,
            include_last_command: false,
        };
        let before = must(shell.capture_state(&scope));
        must(fs::write(&file, "written through resolved alias"));
        let after = must(shell.capture_state(&scope));
        let delta = must(shell.compare_state(
            &before,
            &after,
            Some(&Action::WriteFile {
                id: ActionId::new(),
                path: PathBuf::from("~/Desktop/retina-state-capture-alias-test.txt"),
                content: "written through resolved alias".to_string(),
                overwrite: true,
            }),
        ));

        assert!(matches!(delta.kind, StateDeltaKind::ChangedAsExpected));
        assert_eq!(delta.changed_paths, vec![file.clone()]);

        let _ = fs::remove_file(file);
    }

    #[test]
    fn extract_document_text_reads_pdf_text() {
        let dir = must_tempdir();
        let file = dir.path().join("sample.pdf");
        write_test_pdf_pages(&file, &["hello pdf"]);
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::ExtractDocumentText {
            id: ActionId::new(),
            path: file.clone(),
            max_chars: None,
            page_start: None,
            page_end: None,
        }));
        let ActionResult::DocumentText {
            content, format, ..
        } = result
        else {
            panic!("expected document text");
        };
        assert_eq!(format, "pdf");
        assert!(content.to_lowercase().contains("hello"));
    }

    #[test]
    fn extract_document_text_reads_only_requested_pdf_page() {
        let dir = must_tempdir();
        let file = dir.path().join("multipage.pdf");
        write_test_pdf_pages(&file, &["first page", "second page", "third page"]);
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::ExtractDocumentText {
            id: ActionId::new(),
            path: file.clone(),
            max_chars: None,
            page_start: Some(2),
            page_end: Some(2),
        }));
        let ActionResult::DocumentText {
            content,
            format,
            extraction_method,
            page_range,
            ..
        } = result
        else {
            panic!("expected document text");
        };
        assert_eq!(format, "pdf");
        assert_eq!(extraction_method, "pdf_extract_by_page");
        assert_eq!(
            page_range,
            Some(DocumentPageRange {
                start_page: 2,
                end_page: 2
            })
        );
        assert!(!content.to_lowercase().contains("first"));
        assert!(content.to_lowercase().contains("second"));
        assert!(!content.to_lowercase().contains("third"));
    }

    #[test]
    fn ingest_structured_data_reads_csv_headers_and_rows() {
        let dir = must_tempdir();
        let file = dir.path().join("people.csv");
        must(fs::write(
            &file,
            "name,role,city\nCraig,Engineer,Denver\nEmily,Designer,Boulder\n",
        ));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::IngestStructuredData {
            id: ActionId::new(),
            path: file.clone(),
            max_rows: Some(1),
        }));
        let ActionResult::StructuredData {
            path,
            format,
            headers,
            rows,
            total_rows,
            truncated,
            extraction_method,
        } = result
        else {
            panic!("expected structured data");
        };
        assert_eq!(path, file);
        assert_eq!(format, "csv");
        assert_eq!(headers, vec!["name", "role", "city"]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_number, 1);
        assert_eq!(rows[0].values, vec!["Craig", "Engineer", "Denver"]);
        assert_eq!(total_rows, 2);
        assert!(truncated);
        assert_eq!(extraction_method, "csv_reader");
    }

    fn write_test_pdf_pages(path: &Path, pages: &[&str]) {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.new_object_id();
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            }
        });

        document.objects.insert(
            font_id,
            Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            }),
        );

        let page_ids = pages
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let page_id = document.new_object_id();
                let content_id = document.new_object_id();
                let content = Content {
                    operations: vec![
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
                        Operation::new("Td", vec![72.into(), 100.into()]),
                        Operation::new(
                            "Tj",
                            vec![Object::string_literal(format!(
                                "page {} {}",
                                index + 1,
                                text
                            ))],
                        ),
                        Operation::new("ET", vec![]),
                    ],
                };
                let encoded = content
                    .encode()
                    .unwrap_or_else(|error| panic!("failed to encode PDF test content: {error}"));
                document.objects.insert(
                    content_id,
                    Object::Stream(Stream::new(dictionary! {}, encoded)),
                );
                document.objects.insert(
                    page_id,
                    Object::Dictionary(dictionary! {
                        "Type" => "Page",
                        "Parent" => pages_id,
                        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 144.into()],
                        "Contents" => content_id,
                        "Resources" => resources_id,
                    }),
                );
                page_id
            })
            .collect::<Vec<_>>();

        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::from).collect::<Vec<_>>(),
                "Count" => page_ids.len() as i64,
            }),
        );

        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.compress();
        document
            .save(path)
            .unwrap_or_else(|error| panic!("failed to save test PDF: {error}"));
    }

    fn notebook_fixture() -> String {
        serde_json::json!({
            "cells": [
                {
                    "cell_type": "markdown",
                    "id": "intro",
                    "metadata": {},
                    "source": ["# Hello\n"]
                }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        })
        .to_string()
    }
}
