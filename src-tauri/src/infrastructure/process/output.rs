use crate::common::operation_context::OperationStopped;
use crate::infrastructure::process::child_process;
use std::io;
use std::process::{Output, Stdio};

pub enum ProcessError {
    Io(io::Error),
    Stopped(OperationStopped),
}

struct ProcessGroupGuard(Option<u32>);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            if let Err(error) = child_process::signal_process_group(pid as i32, libc::SIGKILL) {
                log::error!("failed to kill process group: {error}");
            }
        }
    }
}

pub async fn output(
    mut command: tokio::process::Command,
    input: Vec<u8>,
) -> Result<Output, ProcessError> {
    let context = crate::common::operation_context::current();
    context
        .check(std::time::Instant::now())
        .map_err(ProcessError::Stopped)?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    child_process::configure_process_group(&mut command);
    let spawn_guard = crate::infrastructure::process::parent_lifetime::spawn_guard();
    let mut child = command.spawn().map_err(ProcessError::Io)?;
    drop(spawn_guard);
    let mut group = ProcessGroupGuard(child.id());
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let operation = async {
        let write = async {
            stdin.write_all(&input).await?;
            drop(stdin);
            Ok::<_, io::Error>(())
        };
        let read = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            tokio::try_join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err))?;
            Ok::<_, io::Error>((out, err))
        };
        let finish = async {
            #[cfg(unix)]
            {
                child_process::wait_without_reaping(group.0.expect("spawned child id")).await?;
                if let Some(pid) = group.0 {
                    child_process::signal_process_group(pid as i32, libc::SIGKILL)?;
                }
            }
            group.0 = None;
            child.wait().await
        };
        let (_, (stdout, stderr), status) = tokio::try_join!(write, read, finish)?;
        Ok::<_, io::Error>(Output {
            status,
            stdout,
            stderr,
        })
    };
    let result = crate::common::operation_context::wait(&context, operation).await;
    if !matches!(&result, Ok(Ok(_))) {
        drop(group);
        if let Err(error) = child.kill().await {
            log::error!("failed to reap process: {error}");
        }
    }
    result
        .map_err(ProcessError::Stopped)?
        .map_err(ProcessError::Io)
}

#[cfg(test)]
#[path = "output_test.rs"]
mod process_tests;
