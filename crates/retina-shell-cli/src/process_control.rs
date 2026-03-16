use super::*;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

impl CliShell {
    pub(crate) fn run_command(
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
                    observed_paths: Vec::new(),
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
    #[cfg(unix)]
    let mut process = {
        let mut process = Command::new("sh");
        process.arg("-lc").arg(command);
        process
    };

    #[cfg(windows)]
    let mut process = {
        let mut process = Command::new("powershell");
        process
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(command);
        process
    };

    process
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
        Err(KernelError::Execution(
            io::Error::last_os_error().to_string(),
        ))
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
        Ok(())
    }

    #[cfg(not(unix))]
    {
        child.kill()?;
        Ok(())
    }
}
