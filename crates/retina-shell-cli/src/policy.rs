use crate::CliShell;
use retina_traits::Shell;
use retina_types::*;
use std::path::Path;

pub struct ScopedShell<S> {
    inner: S,
    authority: AgentAuthority,
}

impl<S> ScopedShell<S> {
    pub fn new(inner: S, authority: AgentAuthority) -> Self {
        Self { inner, authority }
    }
}

impl<S: Shell> Shell for ScopedShell<S> {
    fn observe(&self) -> Result<WorldState> {
        self.inner.observe()
    }

    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
        for tracked in &scope.tracked_paths {
            ensure_path_allowed(&self.authority, &tracked.path)?;
        }
        self.inner.capture_state(scope)
    }

    fn compare_state(
        &self,
        before: &StateSnapshot,
        after: &StateSnapshot,
        action: Option<&Action>,
    ) -> Result<StateDelta> {
        self.inner.compare_state(before, after, action)
    }

    fn execute(&self, action: &Action) -> Result<ActionResult> {
        validate_action(&self.authority, action)?;
        self.inner.execute(action)
    }

    fn execute_controlled(
        &self,
        action: &Action,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<ActionResult> {
        validate_action(&self.authority, action)?;
        self.inner.execute_controlled(action, control)
    }

    fn constraints(&self) -> &[HardConstraint] {
        self.inner.constraints()
    }

    fn capabilities(&self) -> ShellCapabilities {
        let inner = self.inner.capabilities();
        ShellCapabilities {
            can_execute_commands: inner.can_execute_commands
                && self.authority.allow_command_execution,
            can_read_files: inner.can_read_files && self.authority.allow_file_reads,
            can_write_files: inner.can_write_files && self.authority.allow_file_writes,
            can_search_files: inner.can_search_files && self.authority.allow_file_search,
            can_extract_documents: inner.can_extract_documents && self.authority.allow_file_reads,
            can_write_notes: inner.can_write_notes && self.authority.allow_notes,
            can_respond_text: inner.can_respond_text && self.authority.allow_text_responses,
        }
    }

    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse> {
        self.inner.request_approval(request)
    }

    fn notify(&self, message: &str) -> Result<()> {
        self.inner.notify(message)
    }

    fn request_input(&self, prompt: &str) -> Result<String> {
        self.inner.request_input(prompt)
    }
}

fn validate_action(authority: &AgentAuthority, action: &Action) -> Result<()> {
    match action {
        Action::RunCommand {
            cwd, state_scope, ..
        } => {
            if !authority.allow_command_execution {
                return Err(KernelError::Unsupported(
                    "command execution is not permitted for this agent".to_string(),
                ));
            }
            if let Some(cwd) = cwd {
                ensure_path_allowed(authority, cwd)?;
            }
            for tracked in &state_scope.tracked_paths {
                ensure_path_allowed(authority, &tracked.path)?;
            }
        }
        Action::InspectPath { path, .. }
        | Action::ReadFile { path, .. }
        | Action::IngestStructuredData { path, .. }
        | Action::ExtractDocumentText { path, .. } => {
            if !authority.allow_file_reads {
                return Err(KernelError::Unsupported(
                    "file reads are not permitted for this agent".to_string(),
                ));
            }
            ensure_path_allowed(authority, path)?;
        }
        Action::ListDirectory { path, .. } => {
            if !authority.allow_file_reads {
                return Err(KernelError::Unsupported(
                    "directory inspection is not permitted for this agent".to_string(),
                ));
            }
            ensure_path_allowed(authority, path)?;
        }
        Action::FindFiles { root, .. } | Action::SearchText { root, .. } => {
            if !authority.allow_file_search {
                return Err(KernelError::Unsupported(
                    "file search is not permitted for this agent".to_string(),
                ));
            }
            ensure_path_allowed(authority, root)?;
        }
        Action::WriteFile { path, .. } | Action::AppendFile { path, .. } => {
            if !authority.allow_file_writes {
                return Err(KernelError::Unsupported(
                    "file writes are not permitted for this agent".to_string(),
                ));
            }
            ensure_path_allowed(authority, path)?;
        }
        Action::RecordNote { .. } => {
            if !authority.allow_notes {
                return Err(KernelError::Unsupported(
                    "note recording is not permitted for this agent".to_string(),
                ));
            }
        }
        Action::Respond { .. } => {
            if !authority.allow_text_responses {
                return Err(KernelError::Unsupported(
                    "text responses are not permitted for this agent".to_string(),
                ));
            }
        }
        Action::InspectWorkingDirectory { .. } => {}
    }
    Ok(())
}

fn ensure_path_allowed(authority: &AgentAuthority, path: &Path) -> Result<()> {
    if authority.accessible_roots.is_empty() {
        return Ok(());
    }
    let resolved = CliShell::resolve_path(path)?;
    let allowed = authority
        .accessible_roots
        .iter()
        .map(|root| CliShell::resolve_path(root).unwrap_or_else(|_| root.clone()))
        .any(|root| is_within_root(&resolved, &root));
    if allowed {
        Ok(())
    } else {
        Err(KernelError::Unsupported(format!(
            "path {} is outside this agent's authority scope",
            resolved.display()
        )))
    }
}

fn is_within_root(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn scoped_shell(root: PathBuf, allow_writes: bool) -> ScopedShell<CliShell> {
        ScopedShell::new(
            CliShell::new(),
            AgentAuthority {
                allow_file_writes: allow_writes,
                accessible_roots: vec![root],
                ..AgentAuthority::default()
            },
        )
    }

    #[test]
    fn scoped_shell_blocks_paths_outside_authority_roots() {
        let dir = tempdir().unwrap();
        let shell = scoped_shell(dir.path().to_path_buf(), true);
        let outside = tempdir().unwrap().path().join("note.txt");
        let error = shell
            .execute(&Action::ReadFile {
                id: ActionId::new(),
                path: outside,
                max_bytes: None,
            })
            .unwrap_err();
        assert!(matches!(error, KernelError::Unsupported(_)));
    }

    #[test]
    fn scoped_shell_blocks_writes_when_authority_disallows_them() {
        let dir = tempdir().unwrap();
        let shell = scoped_shell(dir.path().to_path_buf(), false);
        let error = shell
            .execute(&Action::WriteFile {
                id: ActionId::new(),
                path: dir.path().join("note.txt"),
                content: "hello".to_string(),
                overwrite: true,
            })
            .unwrap_err();
        assert!(matches!(error, KernelError::Unsupported(_)));
    }

    #[test]
    fn scoped_shell_allows_actions_within_authority_roots() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.txt");
        fs::write(&file, "hello").unwrap();
        let shell = scoped_shell(dir.path().to_path_buf(), true);
        let result = shell
            .execute(&Action::ReadFile {
                id: ActionId::new(),
                path: file.clone(),
                max_bytes: None,
            })
            .unwrap();
        let ActionResult::FileRead { path, .. } = result else {
            panic!("expected file read");
        };
        assert_eq!(path, file);
    }
}
