mod policy;

pub use policy::ScopedShell;

use blake3::Hasher;
use chrono::{DateTime, Utc};
use pdf_extract::extract_text;
use retina_traits::Shell;
use retina_types::*;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

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

    pub(crate) fn resolve_path(path: &Path) -> Result<PathBuf> {
        if let Some(expanded) = expand_homeish_path(path) {
            return Ok(expanded);
        }
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
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_lowercase();
            let path_text = path.display().to_string().to_lowercase();

            if name.contains(pattern) || path_text.contains(pattern) {
                matches.push(path.clone());
                if matches.len() >= max_results {
                    break;
                }
            }

            if metadata.is_dir() {
                Self::collect_matching_files(&path, pattern, max_results, matches)?;
                continue;
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
        if prefers_document_extraction(&path) {
            return Err(KernelError::Unsupported(format!(
                "read_file is not suitable for {}; use extract_document_text instead",
                path.display()
            )));
        }
        let mut file = fs::File::open(&path)?;
        let limit = max_bytes.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let mut buffer = Vec::new();
        Read::by_ref(&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut buffer)?;
        if looks_binary(&buffer) {
            return Err(KernelError::Unsupported(format!(
                "read_file only supports text-like files; {} appears to be binary",
                path.display()
            )));
        }
        let truncated = buffer.len() > limit;
        if truncated {
            buffer.truncate(limit);
        }
        Ok((String::from_utf8_lossy(&buffer).to_string(), truncated))
    }

    fn extract_document_text(
        path: &Path,
        max_chars: Option<usize>,
    ) -> Result<(String, bool, String)> {
        let path = Self::resolve_path(path)?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();

        let (content, format) = match extension.as_str() {
            "pdf" => (
                extract_text(&path).map_err(|error| KernelError::Execution(error.to_string()))?,
                "pdf".to_string(),
            ),
            "md" | "txt" | "rs" | "toml" | "json" | "yaml" | "yml" => {
                let (content, _) = Self::read_file(&path, None)?;
                (content, extension)
            }
            _ => {
                return Err(KernelError::Unsupported(format!(
                    "document extraction is not supported for {}",
                    path.display()
                )));
            }
        };

        let limit = max_chars.unwrap_or(DEFAULT_MAX_READ_BYTES);
        let mut truncated = false;
        let content = if content.chars().count() > limit {
            truncated = true;
            content.chars().take(limit).collect::<String>()
        } else {
            content
        };
        Ok((content, truncated, format))
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

    fn run_command(
        command: &str,
        cwd: Option<PathBuf>,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<CommandResult> {
        let workdir = cwd.unwrap_or(std::env::current_dir()?);
        let start = Instant::now();
        let mut child = build_shell_command(command, &workdir)?.spawn()?;
        let mut cancelled = false;
        let mut termination = None;

        loop {
            if let Some(status) = child.try_wait()? {
                let (stdout, stderr) = read_child_output(&mut child)?;
                let success = status.success() && !cancelled;
                let result = CommandResult {
                    command: command.to_string(),
                    cwd: workdir,
                    stdout,
                    stderr,
                    exit_code: status.code(),
                    success,
                    duration_ms: start.elapsed().as_millis() as u64,
                    cancelled,
                    termination,
                };
                return Ok(result);
            }

            if control
                .map(ExecutionControlHandle::is_cancel_requested)
                .unwrap_or(false)
            {
                cancelled = true;
                terminate_child_gracefully(&mut child)?;
                if wait_for_exit(&mut child, 1_000)? {
                    termination = Some("terminated gracefully after cancellation".to_string());
                } else {
                    force_kill_child(&mut child)?;
                    let _ = child.wait();
                    termination = Some("force killed after cancellation".to_string());
                }
                continue;
            }

            thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

fn build_shell_command(command: &str, workdir: &Path) -> Result<Command> {
    let mut process = Command::new("sh");
    process
        .arg("-lc")
        .arg(command)
        .current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        process.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
    Ok(process)
}

fn read_child_output(child: &mut Child) -> Result<(String, String)> {
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout)?;
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)?;
    }
    Ok((stdout, stderr))
}

fn wait_for_exit(child: &mut Child, timeout_ms: u64) -> Result<bool> {
    let started = Instant::now();
    while started.elapsed().as_millis() < timeout_ms as u128 {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(false)
}

fn terminate_child_gracefully(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        let result = unsafe { libc::kill(-pid, libc::SIGTERM) };
        if result == 0 {
            return Ok(());
        }
        let result = unsafe { libc::kill(pid, libc::SIGTERM) };
        if result == 0 {
            return Ok(());
        }
        return Err(KernelError::Execution(
            io::Error::last_os_error().to_string(),
        ));
    }

    #[cfg(not(unix))]
    {
        child.kill()?;
        Ok(())
    }
}

fn force_kill_child(child: &mut Child) -> Result<()> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        let result = unsafe { libc::kill(-pid, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        child.kill()?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        child.kill()?;
        Ok(())
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
            Action::ExtractDocumentText {
                path, max_chars, ..
            } => {
                let (content, truncated, format) = Self::extract_document_text(path, *max_chars)?;
                Ok(ActionResult::DocumentText {
                    path: Self::resolve_path(path)?,
                    content,
                    truncated,
                    format,
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
                lock_state(&self.notes)?.push(note.clone());
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

fn expand_homeish_path(path: &Path) -> Option<PathBuf> {
    let raw = path.to_str()?;
    if raw == "~" {
        return dirs::home_dir();
    }
    if let Some(stripped) = raw.strip_prefix("~/") {
        return dirs::home_dir().map(|home| home.join(stripped));
    }

    let first = path
        .components()
        .next()?
        .as_os_str()
        .to_str()?
        .to_lowercase();
    let base = match first.as_str() {
        "desktop" => dirs::home_dir().map(|home| home.join("Desktop")),
        "documents" => dirs::home_dir().map(|home| home.join("Documents")),
        "downloads" => dirs::home_dir().map(|home| home.join("Downloads")),
        _ => None,
    }?;

    let remainder = path.iter().skip(1).collect::<PathBuf>();
    Some(if remainder.as_os_str().is_empty() {
        base
    } else {
        base.join(remainder)
    })
}

fn prefers_document_extraction(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};
    use std::thread;
    use std::time::Duration;
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
    fn controlled_command_can_be_cancelled() {
        let shell = CliShell::new();
        let control = ExecutionControl::new();
        let handle = control.handle();
        let cancel_handle = handle.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel_handle.request_cancel();
        });

        let result = shell
            .execute_controlled(
                &Action::RunCommand {
                    id: ActionId::new(),
                    command: "sleep 5".to_string(),
                    cwd: None,
                    require_approval: false,
                    expect_change: false,
                    state_scope: HashScope::default(),
                },
                Some(&handle),
            )
            .unwrap();
        let ActionResult::Command(result) = result else {
            panic!("expected command result");
        };
        assert!(result.cancelled);
        assert!(!result.success);
        assert!(result.termination.is_some());
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
    fn read_file_rejects_pdf_and_requests_document_tool() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("resume.pdf");
        write_test_pdf(&file, "hello pdf");
        let shell = CliShell::new();
        let error = shell
            .execute(&Action::ReadFile {
                id: ActionId::new(),
                path: file.clone(),
                max_bytes: None,
            })
            .unwrap_err();
        let KernelError::Unsupported(message) = error else {
            panic!("expected unsupported error");
        };
        assert!(message.contains("extract_document_text"));
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

    #[test]
    fn extract_document_text_reads_pdf_text() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("sample.pdf");
        write_test_pdf(&file, "hello pdf");
        let shell = CliShell::new();
        let result = shell
            .execute(&Action::ExtractDocumentText {
                id: ActionId::new(),
                path: file.clone(),
                max_chars: None,
            })
            .unwrap();
        let ActionResult::DocumentText {
            content, format, ..
        } = result
        else {
            panic!("expected document text");
        };
        assert_eq!(format, "pdf");
        assert!(content.to_lowercase().contains("hello"));
    }

    fn write_test_pdf(path: &Path, text: &str) {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();
        let font_id = document.new_object_id();
        let resources_id = document.new_object_id();
        let content_id = document.new_object_id();

        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 24.into()]),
                Operation::new("Td", vec![72.into(), 100.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let encoded = content.encode().unwrap();

        document.objects.insert(
            font_id,
            Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            }),
        );
        document.objects.insert(
            resources_id,
            Object::Dictionary(dictionary! {
                "Font" => dictionary! {
                    "F1" => font_id,
                }
            }),
        );
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
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );

        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.compress();
        document.save(path).unwrap();
    }
}
