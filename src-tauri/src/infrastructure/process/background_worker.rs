use super::attempt::{self, SharedChild};
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};

pub(crate) const FRAME_PREFIX: &str = "releash-background:";

pub(crate) struct BackgroundWorker {
    child: SharedChild,
    stdin: ChildStdin,
    responses: tokio::sync::mpsc::Receiver<io::Result<String>>,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for BackgroundWorker {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl BackgroundWorker {
    pub(crate) fn start(notify_changed: Arc<dyn Fn() + Send + Sync>) -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        #[cfg(not(test))]
        let executable = if cfg!(debug_assertions)
            && executable
                .parent()
                .and_then(|path| path.file_name())
                .is_some_and(|name| name == "deps")
        {
            executable
                .parent()
                .and_then(|path| path.parent())
                .expect("cargo target directory")
                .join("releash-backend")
        } else {
            executable.with_file_name("releash-backend")
        };
        let mut command = Command::new(executable);
        #[cfg(not(test))]
        command.arg("--internal-background-worker");
        #[cfg(test)]
        command
            .args([
                "--exact",
                "adaptor::controller::background_worker::background_worker_tests::worker_entry",
                "--nocapture",
            ])
            .env("RELEASH_TEST_BACKGROUND_WORKER", "1");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = {
            let _spawn = super::parent_lifetime::spawn_guard();
            command.spawn()?
        };
        let stdin = child.stdin.take().expect("worker stdin");
        let stdout = child.stdout.take().expect("worker stdout");
        let child = Arc::new(std::sync::Mutex::new(child));
        attempt::register(&child);
        let (sender, responses) = tokio::sync::mpsc::channel(1);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let Some(frame) = line.strip_prefix(FRAME_PREFIX) else {
                            continue;
                        };
                        if frame == "changed" {
                            notify_changed();
                        } else if sender.send(Ok(frame.to_owned())).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
            reader,
        })
    }

    pub(crate) async fn request(&mut self, request: &[u8]) -> io::Result<String> {
        attempt::register(&self.child);
        self.stdin.write_all(request).await?;
        self.stdin.write_all(b"\n").await?;
        self.responses.recv().await.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "background worker exited before replying",
            )
        })?
    }

    pub(crate) async fn stop(&mut self) -> io::Result<()> {
        attempt::stop(&self.child).await
    }

    #[cfg(test)]
    pub(crate) fn child(&self) -> SharedChild {
        self.child.clone()
    }
}
