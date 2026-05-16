use super::*;

#[derive(Clone, Debug)]
pub(crate) enum LineEndingStyle {
    Lf,
    Crlf,
}

#[derive(Clone, Debug)]
pub(crate) struct FileVersionSnapshot {
    pub(crate) exists: bool,
    pub(crate) size: Option<u64>,
    pub(crate) modified_at: Option<DateTime<Utc>>,
    pub(crate) content_hash: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct StoredReadState {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) normalized_content: String,
    pub(crate) version: FileVersionSnapshot,
    pub(crate) was_partial: bool,
    pub(crate) read_at: DateTime<Utc>,
    pub(crate) line_endings: LineEndingStyle,
}

impl CliShell {
    pub(crate) fn maybe_remember_text_read(
        &self,
        path: &Path,
        content: &str,
        truncated: bool,
    ) -> Result<()> {
        if truncated {
            return Ok(());
        }
        let version = Self::current_file_version(path)?;
        let state = StoredReadState {
            path: path.to_path_buf(),
            content: content.to_string(),
            normalized_content: Self::normalize_line_endings(content),
            version,
            was_partial: false,
            read_at: Utc::now(),
            line_endings: Self::detect_line_endings(content),
        };
        lock_state(&self.read_states)?.insert(path.to_path_buf(), state);
        Ok(())
    }

    pub(crate) fn existing_full_read_state(&self, path: &Path) -> Result<Option<StoredReadState>> {
        Ok(lock_state(&self.read_states)?.get(path).cloned())
    }

    pub(crate) fn require_full_read_for_existing(
        &self,
        path: &Path,
    ) -> Result<Option<StoredReadState>> {
        if !path.exists() {
            return Ok(None);
        }
        let Some(state) = self.existing_full_read_state(path)? else {
            return Err(KernelError::Validation(format!(
                "existing file {} must be read before it can be modified",
                path.display()
            )));
        };
        if state.was_partial {
            return Err(KernelError::Validation(format!(
                "file {} was only partially read; perform a full read before modifying it",
                path.display()
            )));
        }
        Ok(Some(state))
    }

    pub(crate) fn ensure_read_state_is_fresh(
        &self,
        path: &Path,
        state: &StoredReadState,
    ) -> Result<FileVersionSnapshot> {
        if state.path != path {
            return Err(KernelError::Validation(format!(
                "stored read state targeted {} but mutation requested {}",
                state.path.display(),
                path.display()
            )));
        }
        let current = Self::current_file_version(path)?;
        if current.exists != state.version.exists {
            return Err(KernelError::Validation(format!(
                "file {} changed after it was read at {}; re-read before modifying it",
                path.display(),
                state.read_at.to_rfc3339()
            )));
        }

        if current.modified_at == state.version.modified_at && current.size == state.version.size {
            return Ok(current);
        }

        if current.content_hash != state.version.content_hash {
            return Err(KernelError::Validation(format!(
                "file {} changed after it was read at {}; re-read before modifying it",
                path.display(),
                state.read_at.to_rfc3339()
            )));
        }

        Ok(current)
    }

    pub(crate) fn current_file_version(path: &Path) -> Result<FileVersionSnapshot> {
        if !path.exists() {
            return Ok(FileVersionSnapshot {
                exists: false,
                size: None,
                modified_at: None,
                content_hash: None,
            });
        }

        let metadata = fs::metadata(path)?;
        let modified_at = metadata.modified().ok().map(DateTime::<Utc>::from);
        let bytes = fs::read(path)?;
        Ok(FileVersionSnapshot {
            exists: true,
            size: Some(metadata.len()),
            modified_at,
            content_hash: Some(blake3::hash(&bytes).to_hex().to_string()),
        })
    }

    pub(crate) fn detect_line_endings(content: &str) -> LineEndingStyle {
        if content.contains("\r\n") {
            LineEndingStyle::Crlf
        } else {
            LineEndingStyle::Lf
        }
    }

    pub(crate) fn normalize_line_endings(content: &str) -> String {
        content.replace("\r\n", "\n").replace('\r', "\n")
    }

    pub(crate) fn apply_line_endings(content: &str, style: &LineEndingStyle) -> String {
        let normalized = Self::normalize_line_endings(content);
        match style {
            LineEndingStyle::Lf => normalized,
            LineEndingStyle::Crlf => normalized.replace('\n', "\r\n"),
        }
    }
}
