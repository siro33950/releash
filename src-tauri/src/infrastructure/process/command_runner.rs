use std::path::Path;
use std::time::Instant;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::watch;

use super::child_process;

/// 出力キャプチャの上限。ドメイン固有の定数を持ち込まないよう呼び出し側が注入する。
#[derive(Debug, Clone, Copy)]
pub struct OutputLimit {
    pub max_bytes: usize,
    pub truncation_marker: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandRunnerError {
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
pub struct ActiveCommandHandle {
    shutdown_tx: watch::Sender<bool>,
}

impl ActiveCommandHandle {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test() -> Self {
        Self {
            shutdown_tx: watch::channel(false).0,
        }
    }

    pub fn request_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

pub struct RunningCommand {
    label: String,
    output_limit: OutputLimit,
    child: tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    handle: ActiveCommandHandle,
    shutdown_rx: watch::Receiver<bool>,
    started_at: Instant,
}

impl RunningCommand {
    #[cfg(feature = "test-support")]
    pub fn test_label(&self) -> &str {
        &self.label
    }

    pub fn handle(&self) -> ActiveCommandHandle {
        self.handle.clone()
    }

    pub async fn wait(mut self) -> Result<CommandRunOutput, CommandRunnerError> {
        let mut stdout = self.stdout.take();
        let mut stderr = self.stderr.take();
        let limit = self.output_limit;
        let stdout_task = tokio::spawn(async move { read_pipe(stdout.as_mut(), limit).await });
        let stderr_task = tokio::spawn(async move { read_pipe(stderr.as_mut(), limit).await });

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

pub fn spawn_shell_command(
    cwd: impl AsRef<Path>,
    shell_command: &str,
    env: impl IntoIterator<Item = (String, String)>,
    label_prefix: &str,
    output_limit: OutputLimit,
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

    let mut child = {
        let _spawn = super::parent_lifetime::spawn_guard();
        command.spawn().map_err(CommandRunnerError::Spawn)?
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let label = command_label(label_prefix, &cwd);
    Ok(RunningCommand {
        label,
        output_limit,
        child,
        stdout,
        stderr,
        handle: ActiveCommandHandle { shutdown_tx },
        shutdown_rx,
        started_at: Instant::now(),
    })
}

async fn read_pipe<T>(pipe: Option<&mut T>, limit: OutputLimit) -> std::io::Result<String>
where
    T: AsyncReadExt + Unpin,
{
    let Some(pipe) = pipe else {
        return Ok(String::new());
    };
    let mut bytes = Vec::with_capacity(limit.max_bytes + limit.truncation_marker.len());
    let mut buf = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = pipe.read(&mut buf).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.max_bytes.saturating_sub(bytes.len());
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
        bytes.extend_from_slice(limit.truncation_marker.as_bytes());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn command_label(prefix: &str, cwd: &Path) -> String {
    let display = display_cwd(cwd);
    format!("{prefix} in {display}")
}

pub fn display_cwd(cwd: &Path) -> String {
    cwd.to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| cwd.display().to_string())
}
