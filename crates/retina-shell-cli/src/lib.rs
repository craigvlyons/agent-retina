// File boundary: keep lib.rs focused on shell wiring and the top-level CLI shell
// type. Move file/process/state helpers into sibling modules before growth.
mod file_ops;
mod policy;
mod process_control;
mod state_helpers;

pub use policy::ScopedShell;

use crate::state_helpers::{command_fingerprint, path_fingerprint};
use blake3::Hasher;
use chrono::{DateTime, Utc};
use pdf_extract::{extract_text, extract_text_by_pages};
use retina_traits::Shell;
use retina_types::*;
use std::fs::{self, OpenOptions};
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
            .map(|tracked| Self::inspect_path_state(&tracked.path, tracked.include_content))
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
                files: vec![Self::inspect_path_state(path, *include_content)?],
                last_command: lock_state(&self.last_command)?.clone(),
                notes: lock_state(&self.notes)?.clone(),
            })),
            Action::InspectWorkingDirectory { .. } => self.observe().map(ActionResult::Inspection),
            Action::ListDirectory {
                path,
                recursive,
                max_entries,
                ..
            } => Ok(ActionResult::DirectoryListing {
                root: Self::resolve_path(path)?,
                entries: Self::list_directory(
                    path,
                    *recursive,
                    (*max_entries).min(DEFAULT_MAX_LIST_ENTRIES),
                )?,
            }),
            Action::FindFiles {
                root,
                pattern,
                max_results,
                ..
            } => Ok(ActionResult::FileMatches {
                root: Self::resolve_path(root)?,
                pattern: pattern.clone(),
                matches: Self::find_files(
                    root,
                    pattern,
                    (*max_results).min(DEFAULT_MAX_SEARCH_RESULTS),
                )?,
            }),
            Action::SearchText {
                root,
                query,
                max_results,
                ..
            } => Ok(ActionResult::TextSearch {
                root: Self::resolve_path(root)?,
                query: query.clone(),
                matches: Self::search_text(
                    root,
                    query,
                    (*max_results).min(DEFAULT_MAX_SEARCH_RESULTS),
                )?,
            }),
            Action::ReadFile {
                path, max_bytes, ..
            } => {
                let (content, truncated) = Self::read_file(path, *max_bytes)?;
                Ok(ActionResult::FileRead {
                    path: Self::resolve_path(path)?,
                    content,
                    truncated,
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
                let (bytes_written, created, overwritten) =
                    Self::write_file(path, content, *overwrite)?;
                Ok(ActionResult::FileWrite {
                    path: Self::resolve_path(path)?,
                    bytes_written,
                    created,
                    overwritten,
                    appended: false,
                })
            }
            Action::AppendFile { path, content, .. } => {
                let (bytes_written, created) = Self::append_file(path, content)?;
                Ok(ActionResult::FileWrite {
                    path: Self::resolve_path(path)?,
                    bytes_written,
                    created,
                    overwritten: false,
                    appended: true,
                })
            }
            Action::RecordNote { note, .. } => {
                lock_state(&self.notes)?.push(note.clone());
                Ok(ActionResult::NoteRecorded { note: note.clone() })
            }
            Action::Respond { message, .. } => Ok(ActionResult::Response {
                message: message.clone(),
            }),
        }
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
            max_bytes: None,
        }));
        let ActionResult::FileRead {
            path,
            content,
            truncated,
        } = result
        else {
            panic!("expected file read");
        };
        assert_eq!(path, file);
        assert_eq!(content, "hello retina");
        assert!(!truncated);
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
    fn find_files_returns_matches() {
        let dir = must_tempdir();
        let target = dir.path().join("target.txt");
        must(fs::write(&target, "hello"));
        let shell = CliShell::new();
        let result = must(shell.execute(&Action::FindFiles {
            id: ActionId::new(),
            root: dir.path().to_path_buf(),
            pattern: "target".to_string(),
            max_results: 10,
        }));
        let ActionResult::FileMatches { matches, .. } = result else {
            panic!("expected file matches");
        };
        assert_eq!(matches, vec![target]);
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
}
