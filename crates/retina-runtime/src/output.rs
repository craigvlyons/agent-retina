use crate::RuntimeTaskStatus;
use retina_types::{ActionResult, Outcome, Result};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskOutputDelta {
    pub content: String,
    pub new_offset: usize,
    pub truncated: bool,
}

pub fn append_output_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{content}")?;
    Ok(())
}

pub fn read_output_delta(
    path: &Path,
    offset: usize,
    max_bytes: usize,
) -> std::io::Result<TaskOutputDelta> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len() as usize;
    if offset >= size {
        return Ok(TaskOutputDelta {
            content: String::new(),
            new_offset: offset,
            truncated: false,
        });
    }

    let read_len = (size - offset).min(max_bytes);
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut buffer = vec![0; read_len];
    file.read_exact(&mut buffer)?;
    Ok(TaskOutputDelta {
        content: String::from_utf8_lossy(&buffer).to_string(),
        new_offset: offset + read_len,
        truncated: offset + read_len < size,
    })
}

pub fn status_for_outcome(outcome: &Result<Outcome>) -> RuntimeTaskStatus {
    match outcome {
        Ok(Outcome::Success(_)) => RuntimeTaskStatus::Completed,
        Ok(Outcome::Failure(_)) | Err(_) => RuntimeTaskStatus::Failed,
        Ok(Outcome::Blocked(reason)) if reason.contains("cancelled") => RuntimeTaskStatus::Killed,
        Ok(Outcome::Blocked(_)) => RuntimeTaskStatus::Blocked,
    }
}

pub fn outcome_summary(outcome: &Result<Outcome>) -> String {
    match outcome {
        Ok(Outcome::Success(ActionResult::Response { message })) => compact_summary(message),
        Ok(Outcome::Success(result)) => compact_summary(&format!("{result:?}")),
        Ok(Outcome::Failure(reason)) => format!("failed: {reason}"),
        Ok(Outcome::Blocked(reason)) => format!("blocked: {reason}"),
        Err(error) => format!("error: {error}"),
    }
}

pub fn compact_summary(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(240).collect::<String>();
    if normalized.chars().count() > 240 {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_output_delta_from_offset() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("task.output");
        append_output_file(&path, "hello").unwrap();
        append_output_file(&path, "world").unwrap();

        let first = read_output_delta(&path, 0, 6).unwrap();
        assert_eq!(first.content, "hello\n");
        assert_eq!(first.new_offset, 6);
        assert!(first.truncated);

        let second = read_output_delta(&path, first.new_offset, 64).unwrap();
        assert_eq!(second.content, "world\n");
        assert!(!second.truncated);
    }
}
