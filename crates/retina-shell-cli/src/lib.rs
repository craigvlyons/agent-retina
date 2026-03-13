use blake3::Hasher;
use chrono::{DateTime, Utc};
use retina_traits::Shell;
use retina_types::*;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Instant;

const DEFAULT_MAX_READ_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_LIST_ENTRIES: usize = 200;
const DEFAULT_MAX_SEARCH_RESULTS: usize = 50;

pub struct CliShell {
    last_command: Mutex<Option<CommandResult>>,
    notes: Mutex<Vec<String>>,
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

    fn inspect_path_state(path: &Path, include_content: bool) -> Result<PathState> {
        if !path.exists() {
            return Ok(PathState {
                path: path.to_path_buf(),
                exists: false,
                size: None,
                modified_at: None,
                content_hash: None,
            });
        }

        let metadata = fs::metadata(path)?;
        let modified_at = metadata.modified().ok().map(DateTime::<Utc>::from);
        let content_hash = if include_content && metadata.is_file() {
            let bytes = fs::read(path)?;
            Some(blake3::hash(&bytes).to_hex().to_string())
        } else {
            None
        };

        Ok(PathState {
            path: path.to_path_buf(),
            exists: true,
            size: Some(metadata.len()),
            modified_at,
            content_hash,
        })
    }

    fn cwd_hash(path: &Path) -> String {
        blake3::hash(path.display().to_string().as_bytes())
            .to_hex()
            .to_string()
    }

    fn contains_network_command(command: &str) -> bool {
        let lower = command.to_lowercase();
        [
            "curl ", "wget ", "ssh ", "scp ", "nc ", "ping ", "http://", "https://",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn looks_destructive(command: &str) -> bool {
        let lower = command.to_lowercase();
        [
            "rm ",
            "mv ",
            "chmod ",
            "chown ",
            "dd ",
            "mkfs",
            ">: ",
            "truncate ",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn resolve_path(path: &Path) -> Result<PathBuf> {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    }

    fn list_directory(
        path: &Path,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<DirectoryEntry>> {
        let root = Self::resolve_path(path)?;
        let mut entries = Vec::new();
        Self::collect_directory_entries(&root, recursive, max_entries.max(1), &mut entries)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    fn collect_directory_entries(
        dir: &Path,
        recursive: bool,
        max_entries: usize,
        entries: &mut Vec<DirectoryEntry>,
    ) -> Result<()> {
        if entries.len() >= max_entries {
            return Ok(());
        }
        for item in fs::read_dir(dir)? {
            if entries.len() >= max_entries {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            entries.push(DirectoryEntry {
                path: path.clone(),
                is_dir: metadata.is_dir(),
                size: if metadata.is_file() {
                    Some(metadata.len())
                } else {
                    None
                },
            });
            if recursive && metadata.is_dir() {
                Self::collect_directory_entries(&path, true, max_entries, entries)?;
            }
        }
        Ok(())
    }

    fn find_files(root: &Path, pattern: &str, max_results: usize) -> Result<Vec<PathBuf>> {
        let root = Self::resolve_path(root)?;
        let mut matches = Vec::new();
        Self::collect_matching_files(
            &root,
            &pattern.to_lowercase(),
            max_results.max(1),
            &mut matches,
        )?;
        matches.sort();
        Ok(matches)
    }

    fn collect_matching_files(
        root: &Path,
        pattern: &str,
        max_results: usize,
        matches: &mut Vec<PathBuf>,
    ) -> Result<()> {
        if matches.len() >= max_results {
            return Ok(());
        }
        for item in fs::read_dir(root)? {
            if matches.len() >= max_results {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            if metadata.is_dir() {
                Self::collect_matching_files(&path, pattern, max_results, matches)?;
                continue;
            }

            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if name.contains(pattern) || path.display().to_string().to_lowercase().contains(pattern)
            {
                matches.push(path);
            }
        }
        Ok(())
    }

    fn search_text(root: &Path, query: &str, max_results: usize) -> Result<Vec<SearchMatch>> {
        let root = Self::resolve_path(root)?;
        let mut matches = Vec::new();
        Self::collect_text_matches(
            &root,
            &query.to_lowercase(),
            max_results.max(1),
            &mut matches,
        )?;
        Ok(matches)
    }

    fn collect_text_matches(
        root: &Path,
        query: &str,
        max_results: usize,
        matches: &mut Vec<SearchMatch>,
    ) -> Result<()> {
        if matches.len() >= max_results {
            return Ok(());
        }
        for item in fs::read_dir(root)? {
            if matches.len() >= max_results {
                break;
            }
            let item = item?;
            let path = item.path();
            let metadata = item.metadata()?;
            if metadata.is_dir() {
                Self::collect_text_matches(&path, query, max_results, matches)?;
                continue;
            }
            if metadata.len() > 512 * 1024 {
                continue;
            }
            let bytes = fs::read(&path)?;
            if bytes.contains(&0) {
                continue;
            }
            let content = String::from_utf8_lossy(&bytes);
            for (index, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(query) {
                    matches.push(SearchMatch {
                        path: path.clone(),
                        line_number: index + 1,
                        line: line.to_string(),
                    });
                    if matches.len() >= max_results {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn read_file(path: &Path, max_bytes: Option<usize>) -> Result<(String, bool)> {
        let path = Self::resolve_path(path)?;
        let mut file = fs::File::open(path)?;
        let limit = max_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let mut buffer = Vec::new();
        Read::by_ref(&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut buffer)?;
        let truncated = buffer.len() > limit;
        if truncated {
            buffer.truncate(limit);
        }
        Ok((String::from_utf8_lossy(&buffer).to_string(), truncated))
    }

    fn write_file(path: &Path, content: &str, overwrite: bool) -> Result<usize> {
        let path = Self::resolve_path(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() && !overwrite {
            return Err(KernelError::Validation(format!(
                "refusing to overwrite existing file {} without overwrite=true",
                path.display()
            )));
        }
        fs::write(&path, content)?;
        Ok(content.len())
    }

    fn append_file(path: &Path, content: &str) -> Result<usize> {
        let path = Self::resolve_path(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(content.as_bytes())?;
        Ok(content.len())
    }
}

impl Shell for CliShell {
    fn observe(&self) -> Result<WorldState> {
        Ok(WorldState {
            cwd: std::env::current_dir()?,
            files: Vec::new(),
            last_command: self.last_command.lock().unwrap().clone(),
            notes: self.notes.lock().unwrap().clone(),
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
                self.last_command.lock().unwrap().clone()
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
        match action {
            Action::RunCommand {
                command,
                cwd,
                require_approval,
                ..
            } => {
                if Self::contains_network_command(command) {
                    return Err(KernelError::Unsupported(
                        "network commands are not shell-native actions in v1".to_string(),
                    ));
                }
                if Self::looks_destructive(command) && !require_approval {
                    return Err(KernelError::ApprovalDenied(
                        "destructive command blocked until explicitly approved".to_string(),
                    ));
                }

                let workdir = cwd.clone().unwrap_or(std::env::current_dir()?);
                let start = Instant::now();
                let output = Command::new("sh")
                    .arg("-lc")
                    .arg(command)
                    .current_dir(&workdir)
                    .output()?;
                let result = CommandResult {
                    command: command.clone(),
                    cwd: workdir,
                    stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    exit_code: output.status.code(),
                    success: output.status.success(),
                    duration_ms: start.elapsed().as_millis() as u64,
                };
                *self.last_command.lock().unwrap() = Some(result.clone());
                Ok(ActionResult::Command(result))
            }
            Action::InspectPath {
                path,
                include_content,
                ..
            } => Ok(ActionResult::Inspection(WorldState {
                cwd: std::env::current_dir()?,
                files: vec![Self::inspect_path_state(path, *include_content)?],
                last_command: self.last_command.lock().unwrap().clone(),
                notes: self.notes.lock().unwrap().clone(),
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
            Action::WriteFile {
                path,
                content,
                overwrite,
                ..
            } => {
                let bytes_written = Self::write_file(path, content, *overwrite)?;
                Ok(ActionResult::FileWrite {
                    path: Self::resolve_path(path)?,
                    bytes_written,
                    appended: false,
                })
            }
            Action::AppendFile { path, content, .. } => {
                let bytes_written = Self::append_file(path, content)?;
                Ok(ActionResult::FileWrite {
                    path: Self::resolve_path(path)?,
                    bytes_written,
                    appended: true,
                })
            }
            Action::RecordNote { note, .. } => {
                self.notes.lock().unwrap().push(note.clone());
                Ok(ActionResult::NoteRecorded { note: note.clone() })
            }
            Action::Respond { message, .. } => Ok(ActionResult::Response {
                message: message.clone(),
            }),
        }
    }

    fn constraints(&self) -> &[HardConstraint] {
        static CONSTRAINTS: [HardConstraint; 2] = [
            HardConstraint::NoNetworkShellActions,
            HardConstraint::DestructiveOperationsRequireApproval,
        ];
        &CONSTRAINTS
    }

    fn capabilities(&self) -> ShellCapabilities {
        ShellCapabilities {
            can_execute_commands: true,
            can_read_files: true,
            can_write_files: true,
            can_search_files: true,
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

fn path_fingerprint(path_state: &PathState) -> String {
    format!(
        "{}:{}:{:?}:{:?}",
        path_state.exists,
        path_state.size.unwrap_or_default(),
        path_state.modified_at,
        path_state.content_hash
    )
}

fn command_fingerprint(result: &CommandResult) -> String {
    let mut hasher = Hasher::new();
    hasher.update(result.command.as_bytes());
    hasher.update(result.stdout.as_bytes());
    hasher.update(result.stderr.as_bytes());
    hasher.update(&result.exit_code.unwrap_or_default().to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn command_execution_captures_output() {
        let shell = CliShell::new();
        let result = shell
            .execute(&Action::RunCommand {
                id: ActionId::new(),
                command: "printf 'hello'".to_string(),
                cwd: None,
                require_approval: false,
                expect_change: false,
                state_scope: HashScope::default(),
            })
            .unwrap();
        let ActionResult::Command(result) = result else {
            panic!("expected command result");
        };
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.duration_ms <= 5_000);
    }

    #[test]
    fn read_file_returns_content() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.txt");
        fs::write(&file, "hello retina").unwrap();
        let shell = CliShell::new();
        let result = shell
            .execute(&Action::ReadFile {
                id: ActionId::new(),
                path: file.clone(),
                max_bytes: None,
            })
            .unwrap();
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
    fn find_files_returns_matches() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, "hello").unwrap();
        let shell = CliShell::new();
        let result = shell
            .execute(&Action::FindFiles {
                id: ActionId::new(),
                root: dir.path().to_path_buf(),
                pattern: "target".to_string(),
                max_results: 10,
            })
            .unwrap();
        let ActionResult::FileMatches { matches, .. } = result else {
            panic!("expected file matches");
        };
        assert_eq!(matches, vec![target]);
    }

    #[test]
    fn state_capture_detects_file_change() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.txt");
        fs::write(&file, "before").unwrap();
        let shell = CliShell::new();
        let scope = HashScope {
            tracked_paths: vec![TrackedPath {
                path: file.clone(),
                include_content: true,
            }],
            include_working_directory: false,
            include_last_command: false,
        };
        let before = shell.capture_state(&scope).unwrap();
        fs::write(&file, "after").unwrap();
        let after = shell.capture_state(&scope).unwrap();
        let delta = shell
            .compare_state(
                &before,
                &after,
                Some(&Action::WriteFile {
                    id: ActionId::new(),
                    path: file.clone(),
                    content: "after".to_string(),
                    overwrite: true,
                    require_approval: true,
                }),
            )
            .unwrap();
        assert!(matches!(delta.kind, StateDeltaKind::ChangedAsExpected));
        assert_eq!(delta.changed_paths, vec![file]);
    }

    #[test]
    fn state_capture_reports_unchanged_for_noop() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.txt");
        fs::write(&file, "same").unwrap();
        let shell = CliShell::new();
        let scope = HashScope {
            tracked_paths: vec![TrackedPath {
                path: file,
                include_content: true,
            }],
            include_working_directory: false,
            include_last_command: false,
        };
        let before = shell.capture_state(&scope).unwrap();
        let after = shell.capture_state(&scope).unwrap();
        let delta = shell.compare_state(&before, &after, None).unwrap();
        assert!(matches!(delta.kind, StateDeltaKind::Unchanged));
    }
}
