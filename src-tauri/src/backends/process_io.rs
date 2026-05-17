use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub(crate) struct CommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// 上限バイト数まで非同期で読み出す。上限到達後の残りは破棄する（プロセスは継続）。
pub(crate) async fn read_with_limit<R>(
    reader: &mut R,
    max_bytes: usize,
    context: &str,
) -> Result<Vec<u8>, String>
where
    R: AsyncReadExt + Unpin,
{
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if buf.len() >= max_bytes {
            let mut sink = [0u8; 8192];
            while reader
                .read(&mut sink)
                .await
                .map_err(|e| format!("{context} 出力読み出し失敗: {e}"))?
                > 0
            {}
            break;
        }
        let n = reader
            .read(&mut chunk)
            .await
            .map_err(|e| format!("{context} 出力読み出し失敗: {e}"))?;
        if n == 0 {
            break;
        }
        let take = n.min(max_bytes - buf.len());
        buf.extend_from_slice(&chunk[..take]);
    }
    Ok(buf)
}

struct ProcessTreeGuard {
    #[cfg(unix)]
    pgid: Option<u32>,
    armed: bool,
}

impl ProcessTreeGuard {
    fn new() -> Self {
        Self {
            #[cfg(unix)]
            pgid: None,
            armed: true,
        }
    }

    #[cfg(unix)]
    fn set_child_id(&mut self, child_id: Option<u32>) {
        self.pgid = child_id;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn kill_now(&mut self) {
        if !self.armed {
            return;
        }
        // PGID 再利用時に二度 kill しないよう、kill 前に disarm する。
        // これにより以降の kill_now（Drop 経由含む）は no-op となる。
        self.armed = false;
        kill_process_tree(
            #[cfg(unix)]
            self.pgid,
        );
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}

#[cfg(unix)]
fn configure_process_tree(cmd: &mut Command) {
    cmd.as_std_mut().process_group(0);
}

#[cfg(not(unix))]
fn configure_process_tree(_cmd: &mut Command) {}

#[cfg(unix)]
fn kill_process_tree(pgid: Option<u32>) {
    let Some(pgid) = pgid else {
        return;
    };
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_process_tree() {}

pub(crate) async fn run_command_with_output_limit(
    mut cmd: Command,
    timeout: Duration,
    max_bytes: usize,
    output_context: &str,
    spawn_context: &str,
    wait_context: &str,
    timeout_message: String,
) -> Result<CommandOutput, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_tree(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| format!("{spawn_context}: {e}"))?;
    let mut process_tree_guard = ProcessTreeGuard::new();
    #[cfg(unix)]
    process_tree_guard.set_child_id(child.id());

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let stdout_task = async {
        match stdout_handle {
            Some(mut h) => read_with_limit(&mut h, max_bytes, output_context).await,
            None => Ok(Vec::new()),
        }
    };
    let stderr_task = async {
        match stderr_handle {
            Some(mut h) => read_with_limit(&mut h, max_bytes, output_context).await,
            None => Ok(Vec::new()),
        }
    };
    let wait_fut = async {
        let (stdout_res, stderr_res) = tokio::join!(stdout_task, stderr_task);
        let stdout = stdout_res?;
        let stderr = stderr_res?;
        let status = child
            .wait()
            .await
            .map_err(|e| format!("{wait_context}: {e}"))?;
        process_tree_guard.disarm();
        Ok::<_, String>(CommandOutput {
            status,
            stdout,
            stderr,
        })
    };

    match tokio::time::timeout(timeout, wait_fut).await {
        Ok(result) => result,
        Err(_) => {
            process_tree_guard.kill_now();
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
            Err(timeout_message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_command_with_output_limit_returns_timeout_message() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("sleep 1");

        let result = run_command_with_output_limit(
            cmd,
            Duration::from_millis(10),
            1024,
            "test command",
            "spawn failed",
            "wait failed",
            "test timeout".to_string(),
        )
        .await;

        match result {
            Ok(_) => panic!("expected timeout error"),
            Err(err) => assert_eq!(err, "test timeout"),
        }
    }
}
