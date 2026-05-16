use super::*;
use crate::read_state::{FileVersionSnapshot, StoredReadState};
use tempfile::NamedTempFile;

impl CliShell {
    pub(crate) fn write_text_atomically(
        &self,
        path: &Path,
        content: &str,
        prior_read: Option<&StoredReadState>,
    ) -> Result<(usize, FileVersionSnapshot, String)> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let final_content = prior_read
            .map(|state| Self::apply_line_endings(content, &state.line_endings))
            .unwrap_or_else(|| content.to_string());

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = NamedTempFile::new_in(parent).map_err(|error| {
            KernelError::Execution(format!(
                "failed to create temporary file next to {}: {error}",
                path.display()
            ))
        })?;
        temp.write_all(final_content.as_bytes()).map_err(|error| {
            KernelError::Execution(format!(
                "failed to write temporary file for {}: {error}",
                path.display()
            ))
        })?;
        temp.flush().map_err(|error| {
            KernelError::Execution(format!(
                "failed to flush temporary file for {}: {error}",
                path.display()
            ))
        })?;
        temp.persist(path).map_err(|error| {
            KernelError::Execution(format!(
                "failed to persist temporary file to {}: {}",
                path.display(),
                error.error
            ))
        })?;

        let version = Self::current_file_version(path)?;
        Ok((final_content.len(), version, final_content))
    }
}
