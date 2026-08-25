use std::path::Path;
use std::time::Instant;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::watch;

use super::child_process;

/// 出力キャプチャの上限。ドメイン固有の定数を持ち込まないよう呼び出し側が注入する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputLimit {
    pub(crate) max_bytes: usize,
    pub(crate) truncation_marker: &'static str,
}

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
    output_limit: OutputLimit,
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

pub(crate) fn spawn_shell_command(
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

    let mut child = command.spawn().map_err(CommandRunnerError::Spawn)?;
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

fn display_cwd(cwd: &Path) -> String {
    cwd.to_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| cwd.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TEST_LABEL: &str = "workflow command";
    const TEST_LIMIT: OutputLimit = OutputLimit {
        max_bytes: 100 * 1024,
        truncation_marker: "... (truncated)",
    };

    #[tokio::test]
    async fn shell_command_runs_in_cwd_and_captures_output_and_status() {
        let cwd = TempDir::new().unwrap();
        let canonical_cwd = std::fs::canonicalize(cwd.path()).unwrap();

        let output = spawn_shell_command(
            cwd.path(),
            "printf '%s' \"$PWD\"; printf '%s' err >&2; exit 7",
            std::iter::empty::<(String, String)>(),
            TEST_LABEL,
            TEST_LIMIT,
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
            TEST_LABEL,
            TEST_LIMIT,
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
            TEST_LABEL,
            TEST_LIMIT,
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
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 0);
        let marker = TEST_LIMIT.truncation_marker;
        assert!(output.stdout.ends_with(marker));
        assert!(output.stderr.ends_with(marker));
        assert!(output.stdout.len() <= TEST_LIMIT.max_bytes + marker.len());
        assert!(output.stderr.len() <= TEST_LIMIT.max_bytes + marker.len());
    }

    #[tokio::test]
    async fn test_shell環境変数_引用付き参照は値を再解釈せず元の内容を渡す() {
        // Given
        let cwd = TempDir::new().unwrap();
        let marker = cwd.path().join("must-not-exist");
        let value = format!(
            "single' double\" `touch {}`\n$HOME; touch {}",
            marker.display(),
            marker.display()
        );

        // When
        let output = spawn_shell_command(
            cwd.path(),
            "printf '%s' \"$DOC\"",
            [("DOC".to_string(), value.clone())],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        // Then
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, value);
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn test_shell環境変数_引用なし参照でも値のshell構文はcommandにならない() {
        // Given
        let cwd = TempDir::new().unwrap();
        let marker = cwd.path().join("must-not-exist");
        let value = format!(
            "one two; touch {} `touch {}`",
            marker.display(),
            marker.display()
        );

        // When
        let output = spawn_shell_command(
            cwd.path(),
            "printf '<%s>\\n' $DOC",
            [("DOC".to_string(), value)],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        // Then
        assert_eq!(output.exit_code, 0);
        assert!(!marker.exists());
        assert!(output.stdout.contains("<two;>"));
        assert!(output.stdout.contains("<`touch>"));
    }

    #[test]
    fn test_shell環境変数_nulを含む値は既存spawn_errorになる() {
        let cwd = TempDir::new().unwrap();

        let error = spawn_shell_command(
            cwd.path(),
            "true",
            [("DOC".to_string(), "before\0after".to_string())],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .err()
        .expect("NULを含む環境変数ではprocessを起動できない");

        assert!(matches!(error, CommandRunnerError::Spawn(_)));
    }

    #[test]
    fn test_shell環境変数_platform上限超過は既存spawn_errorになる() {
        let cwd = TempDir::new().unwrap();
        let value = "x".repeat(2 * 1024 * 1024);

        let error = spawn_shell_command(
            cwd.path(),
            "true",
            [("DOC".to_string(), value)],
            TEST_LABEL,
            TEST_LIMIT,
        )
        .err()
        .expect("platform上限を超える環境変数ではprocessを起動できない");

        assert!(matches!(error, CommandRunnerError::Spawn(_)));
    }
}
