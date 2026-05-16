use crate::output::{outcome_summary, status_for_outcome};
use crate::supervisor::persist_snapshot;
use crate::{RuntimeTask, RuntimeTaskHandle, RuntimeTaskKind, TaskSupervisor};
use retina_types::{ActionResult, CommandResult, ExecutionControlHandle, Outcome, Result, Task};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub struct CommandTaskInput {
    pub task: Task,
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub control: ExecutionControlHandle,
}

impl TaskSupervisor {
    pub fn spawn_command(&self, input: CommandTaskInput) -> RuntimeTaskHandle {
        let control = input.control.clone();
        let output_path = Some(self.output_path_for(&input.task));
        self.registry().register(RuntimeTask::new(
            &input.task,
            RuntimeTaskKind::Command,
            output_path,
        ));
        persist_snapshot(&self.registry(), self.store(), &input.task.id);
        self.registry()
            .mark_running(&input.task.id, "running command");
        persist_snapshot(&self.registry(), self.store(), &input.task.id);

        let registry = self.registry();
        let store = self.store_arc();
        let task_id = input.task.id.clone();
        let task = input.task.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let outcome = run_command_task(input.command, input.cwd, input.control);
            registry.append_output(&task_id, &command_output_record(&outcome));
            let summary = outcome_summary(&outcome);
            registry.mark_terminal(&task_id, status_for_outcome(&outcome), summary);
            persist_snapshot(&registry, store.as_deref(), &task_id);
            let _ = sender.send(outcome);
        });

        RuntimeTaskHandle::new(task, control, receiver, self.registry())
    }
}

fn run_command_task(
    command: String,
    cwd: Option<PathBuf>,
    control: ExecutionControlHandle,
) -> Result<Outcome> {
    let start = Instant::now();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let command_cwd = cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut child = Command::new(shell)
        .arg("-lc")
        .arg(&command)
        .current_dir(&command_cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| retina_types::KernelError::Execution(error.to_string()))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || read_pipe(stdout));
    let stderr_reader = thread::spawn(move || read_pipe(stderr));
    let mut cancelled = false;
    let exit_code = loop {
        if control.is_cancel_requested() {
            cancelled = true;
            let _ = child.kill();
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| retina_types::KernelError::Execution(error.to_string()))?
        {
            break status.code();
        }
        thread::sleep(Duration::from_millis(100));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Outcome::Success(ActionResult::Command(CommandResult {
        command,
        cwd: command_cwd,
        stdout,
        stderr,
        exit_code,
        success: !cancelled && exit_code == Some(0),
        duration_ms: start.elapsed().as_millis() as u64,
        cancelled,
        termination: if cancelled {
            Some("command cancelled".to_string())
        } else {
            None
        },
        observed_paths: Vec::new(),
    })))
}

fn read_pipe(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut output = String::new();
    let _ = pipe.read_to_string(&mut output);
    output
}

fn command_output_record(outcome: &Result<Outcome>) -> String {
    let Ok(Outcome::Success(ActionResult::Command(result))) = outcome else {
        return outcome_summary(outcome);
    };

    let mut record = format!(
        "command: {}\ncwd: {}\nexit_code: {:?}\nsuccess: {}\nduration_ms: {}\ncancelled: {}",
        result.command,
        result.cwd.display(),
        result.exit_code,
        result.success,
        result.duration_ms,
        result.cancelled
    );
    if let Some(termination) = &result.termination {
        record.push_str(&format!("\ntermination: {termination}"));
    }
    if !result.stdout.is_empty() {
        record.push_str("\nstdout:\n");
        record.push_str(&result.stdout);
    }
    if !result.stderr.is_empty() {
        record.push_str("\nstderr:\n");
        record.push_str(&result.stderr);
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use retina_types::{AgentId, ExecutionControl};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn command_task_runs_in_background_and_records_output() {
        let dir = tempdir().unwrap();
        let supervisor = TaskSupervisor::new(dir.path().to_path_buf());
        let control = ExecutionControl::new();
        let task = Task::new(AgentId::new(), "run echo");
        let handle = supervisor.spawn_command(CommandTaskInput {
            task,
            command: "printf hello".to_string(),
            cwd: None,
            control: control.handle(),
        });

        let outcome = handle.recv().unwrap();
        let Outcome::Success(ActionResult::Command(result)) = outcome else {
            panic!("expected command result");
        };
        assert_eq!(result.stdout, "hello");
        let snapshot = supervisor
            .registry()
            .snapshots()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(snapshot.task_kind, RuntimeTaskKind::Command);
        assert!(snapshot.status.is_terminal());
        let output_path = snapshot.output_path.unwrap();
        assert!(output_path.exists());
        let recorded_output = fs::read_to_string(output_path).unwrap();
        assert!(recorded_output.contains("stdout:\nhello"));
    }
}
