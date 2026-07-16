use std::path::Path;
use std::time::Instant;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::watch;

use super::child_process;
use crate::adaptor::gateway::workflow::output_limit::{MAX_OUTPUT_SIZE, TRUNCATION_MARKER};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandRunOutput {
    pub(crate) exit_code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandRunnerError {
    #[error("failed to spawn command: {0}")]
    Spawn(std::io::Error),
    #[error("failed to wait for command: {0}")]
    Wait(std::io::Error),
    #[error("failed to read command output: {0}")]
    Output(std::io::Error),
    #[error("command was cancelled")]
    Cancelled,
}

#[derive(Clone)]
pub(crate) struct ActiveCommandHandle {
    shutdown_tx: watch::Sender<bool>,
}

impl ActiveCommandHandle {
    pub(crate) fn request_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

pub(crate) struct RunningCommand {
    label: String,
    child: tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    handle: ActiveCommandHandle,
    shutdown_rx: watch::Receiver<bool>,
    started_at: Instant,
}

impl RunningCommand {
    pub(crate) fn handle(&self) -> ActiveCommandHandle {
        self.handle.clone()
    }

    pub(crate) async fn wait(mut self) -> Result<CommandRunOutput, CommandRunnerError> {
        let mut stdout = self.stdout.take();
        let mut stderr = self.stderr.take();
        let stdout_task = tokio::spawn(async move { read_pipe(stdout.as_mut()).await });
        let stderr_task = tokio::spawn(async move { read_pipe(stderr.as_mut()).await });

        let status = tokio::select! {
            status = self.child.wait() => {
                status.map_err(CommandRunnerError::Wait)?
            }
            changed = self.shutdown_rx.changed() => {
                if changed.is_ok() && *self.shutdown_rx.borrow() {
                    child_process::staged_shutdown(&mut self.child, &self.label).await;
                    return Err(CommandRunnerError::Cancelled);
                }
                self.child.wait().await.map_err(CommandRunnerError::Wait)?
            }
        };

        let stdout = stdout_task
            .await
            .map_err(|err| CommandRunnerError::Output(std::io::Error::other(err)))?
            .map_err(CommandRunnerError::Output)?;
        let stderr = stderr_task
            .await
            .map_err(|err| CommandRunnerError::Output(std::io::Error::other(err)))?
            .map_err(CommandRunnerError::Output)?;
        let exit_code = status.code().unwrap_or(-1);
        Ok(CommandRunOutput {
            exit_code,
            stdout,
            stderr,
            duration_ms: self
                .started_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        })
    }
}

pub(crate) fn spawn_shell_command(
    cwd: impl AsRef<Path>,
    shell_command: &str,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<RunningCommand, CommandRunnerError> {
    let cwd = cwd.as_ref().to_path_buf();
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(shell_command)
        .current_dir(&cwd)
        .envs(env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    child_process::configure_process_group(&mut command);

    let mut child = command.spawn().map_err(CommandRunnerError::Spawn)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let label = command_label(&cwd);
    Ok(RunningCommand {
        label,
        child,
        stdout,
        stderr,
        handle: ActiveCommandHandle { shutdown_tx },
        shutdown_rx,
        started_at: Instant::now(),
    })
}

async fn read_pipe<T>(pipe: Option<&mut T>) -> std::io::Result<String>
where
    T: AsyncReadExt + Unpin,
{
    let Some(pipe) = pipe else {
        return Ok(String::new());
    };
    let mut bytes = Vec::with_capacity(MAX_OUTPUT_SIZE + TRUNCATION_MARKER.len());
    let mut buf = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = pipe.read(&mut buf).await?;
        if read == 0 {
            break;
        }
        let remaining = MAX_OUTPUT_SIZE.saturating_sub(bytes.len());
        if remaining > 0 {
            let keep = remaining.min(read);
            bytes.extend_from_slice(&buf[..keep]);
            if keep < read {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    if truncated {
        bytes.extend_from_slice(TRUNCATION_MARKER.as_bytes());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn command_label(cwd: &Path) -> String {
    let display = display_cwd(cwd);
    format!("workflow command in {display}")
}

fn display_cwd(cwd: &Path) -> String {
    cwd.to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| cwd.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn shell_command_runs_in_cwd_and_captures_output_and_status() {
        let cwd = TempDir::new().unwrap();
        let canonical_cwd = std::fs::canonicalize(cwd.path()).unwrap();

        let output = spawn_shell_command(
            cwd.path(),
            "printf '%s' \"$PWD\"; printf '%s' err >&2; exit 7",
            std::iter::empty::<(String, String)>(),
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 7);
        assert_eq!(output.stdout, canonical_cwd.to_string_lossy());
        assert_eq!(output.stderr, "err");
        assert!(output.duration_ms < 60_000);
    }

    #[tokio::test]
    async fn running_command_label_does_not_retain_shell_command() {
        let cwd = TempDir::new().unwrap();
        let secret_command = "printf '%s' label-secret-sentinel";

        let running = spawn_shell_command(
            cwd.path(),
            secret_command,
            std::iter::empty::<(String, String)>(),
        )
        .unwrap();

        assert_eq!(
            running.label,
            format!("workflow command in {}", display_cwd(cwd.path()))
        );
        assert!(!running.label.contains(secret_command));
        assert!(!running.label.contains("label-secret-sentinel"));

        running.wait().await.unwrap();
    }

    #[tokio::test]
    async fn shell_command_cancellation_returns_cancelled() {
        let cwd = TempDir::new().unwrap();
        let running = spawn_shell_command(
            cwd.path(),
            "sleep 30",
            std::iter::empty::<(String, String)>(),
        )
        .unwrap();
        let handle = running.handle();

        let waiter = tokio::spawn(async move { running.wait().await });
        handle.request_shutdown();
        let err = waiter.await.unwrap().unwrap_err();

        assert!(matches!(err, CommandRunnerError::Cancelled));
    }

    #[tokio::test]
    async fn shell_command_output_capture_is_bounded_and_drains_to_exit() {
        let cwd = TempDir::new().unwrap();
        let output = spawn_shell_command(
            cwd.path(),
            "head -c 200000 /dev/zero | tr '\\0' x; head -c 200000 /dev/zero | tr '\\0' e >&2",
            std::iter::empty::<(String, String)>(),
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.ends_with(TRUNCATION_MARKER));
        assert!(output.stderr.ends_with(TRUNCATION_MARKER));
        assert!(output.stdout.len() <= MAX_OUTPUT_SIZE + TRUNCATION_MARKER.len());
        assert!(output.stderr.len() <= MAX_OUTPUT_SIZE + TRUNCATION_MARKER.len());
    }
}
