use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::{mpsc, Arc};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

pub(crate) struct NativePtySpawnConfig {
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) cwd: Option<String>,
    pub(crate) shell: String,
    pub(crate) integration_dir: Option<std::path::PathBuf>,
    pub(crate) runtime_id: u64,
    pub(crate) extra_env: Vec<(String, String)>,
    pub(crate) process: Option<NativePtyProcessConfig>,
}

pub(crate) struct NativePtyProcessConfig {
    pub(crate) executable: OsString,
    pub(crate) arguments: Vec<String>,
    pub(crate) environment: Vec<(String, String)>,
}

pub(crate) struct SpawnedNativePty {
    pub(crate) runtime: NativePtyRuntime,
    pub(crate) output: NativePtyOutput,
}

#[derive(Clone)]
pub struct NativePtyRuntime {
    input: mpsc::SyncSender<Vec<u8>>,
    input_error: Arc<Mutex<Option<NativePtyError>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn NativePtyResizer + Send>>>,
}

impl NativePtyRuntime {
    pub(crate) fn write(&self, data: &[u8]) -> Result<(), NativePtyError> {
        if let Some(error) = self.input_error.lock().as_ref() {
            return Err(error.clone());
        }
        self.input
            .try_send(data.to_vec())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => NativePtyError {
                    kind: std::io::ErrorKind::WouldBlock,
                    message: "PTY input queue is full".into(),
                },
                mpsc::TrySendError::Disconnected(_) => self
                    .input_error
                    .lock()
                    .clone()
                    .unwrap_or_else(|| NativePtyError {
                        kind: std::io::ErrorKind::BrokenPipe,
                        message: "PTY input writer is unavailable".into(),
                    }),
            })
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), NativePtyError> {
        self.resizer.lock().resize(rows, cols)
    }

    pub(crate) fn kill(&self) -> Result<(), NativePtyError> {
        self.killer
            .lock()
            .kill()
            .map_err(|error| NativePtyError::io(error, "Failed to kill PTY"))
    }

    fn new(
        mut writer: Box<dyn Write + Send>,
        killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
        resizer: Box<dyn NativePtyResizer + Send>,
    ) -> Self {
        const INPUT_QUEUE_CAPACITY: usize = 1024;
        let (input, receiver) = mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE_CAPACITY);
        let input_error = Arc::new(Mutex::new(None));
        let worker_error = Arc::clone(&input_error);
        std::thread::spawn(move || {
            while let Ok(mut data) = receiver.recv() {
                while let Ok(next) = receiver.try_recv() {
                    data.extend_from_slice(&next);
                }
                let result = writer
                    .write_all(&data)
                    .map_err(|error| NativePtyError::io(error, "Failed to write to PTY"))
                    .and_then(|()| {
                        writer
                            .flush()
                            .map_err(|error| NativePtyError::io(error, "Failed to flush PTY"))
                    });
                if let Err(error) = result {
                    *worker_error.lock() = Some(error);
                    break;
                }
            }
        });
        Self {
            input,
            input_error,
            killer: Arc::new(Mutex::new(killer)),
            resizer: Arc::new(Mutex::new(resizer)),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_parts(
        writer: Box<dyn Write + Send>,
        killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
        resizer: Box<dyn NativePtyResizer + Send>,
    ) -> Self {
        Self::new(writer, killer, resizer)
    }
}

pub(crate) struct NativePtyOutput {
    reader: Box<dyn Read + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl NativePtyOutput {
    #[cfg(test)]
    pub(crate) fn from_parts(
        reader: Box<dyn Read + Send>,
        child: Box<dyn portable_pty::Child + Send + Sync>,
    ) -> Self {
        Self { reader, child }
    }

    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buffer)
    }

    pub(crate) fn wait(mut self) -> Result<Option<i32>, String> {
        self.child
            .wait()
            .map(|status| Some(status.exit_code() as i32))
            .map_err(|error| format!("Failed to wait for PTY child: {error}"))
    }
}

pub trait NativePtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), NativePtyError>;
}

struct PortablePtyResizer {
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl NativePtyResizer for PortablePtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), NativePtyError> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| NativePtyError::external(error, "Failed to resize PTY"))
    }
}

pub(crate) struct NativePtySystem;

fn configure_terminal_environment(command: &mut CommandBuilder, managed_process: bool) {
    if managed_process {
        command.env_remove("NO_COLOR");
    }

    #[cfg(not(target_os = "windows"))]
    {
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        if std::env::var("LANG").is_err() {
            command.env("LANG", "en_US.UTF-8");
        }
    }
}

impl NativePtySystem {
    pub(crate) fn spawn(
        &self,
        config: NativePtySpawnConfig,
    ) -> Result<SpawnedNativePty, NativePtyError> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| NativePtyError::external(error, "Failed to open PTY"))?;

        let managed_process = config.process.is_some();
        let mut command = if let Some(process) = config.process {
            let mut command = CommandBuilder::new(process.executable);
            for argument in process.arguments {
                command.arg(argument);
            }
            for (key, value) in process.environment {
                command.env(key, value);
            }
            command
        } else if let Some(integration_dir) = config.integration_dir {
            if config.shell.ends_with("/bash") {
                let mut command = CommandBuilder::new(&config.shell);
                command.arg("--rcfile");
                command.arg(integration_dir.join("bash-init.sh"));
                command
            } else if config.shell.ends_with("/zsh") {
                let mut command = CommandBuilder::new(&config.shell);
                let user_zdotdir = std::env::var("ZDOTDIR")
                    .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
                command.env("RELEASH_USER_ZDOTDIR", user_zdotdir);
                command.env("ZDOTDIR", integration_dir.join("zsh"));
                command
            } else if config.shell.ends_with("/fish") {
                let mut command = CommandBuilder::new(&config.shell);
                command.arg("-C");
                command.arg(format!(
                    "source '{}'",
                    integration_dir.join("fish-init.fish").display()
                ));
                command
            } else {
                CommandBuilder::new_default_prog()
            }
        } else {
            CommandBuilder::new_default_prog()
        };

        configure_terminal_environment(&mut command, managed_process);

        command.env("RELEASH_PTY_ID", config.runtime_id.to_string());
        for (key, value) in config.extra_env {
            command.env(key, value);
        }
        if let Some(cwd) = config.cwd {
            command.cwd(cwd);
        }

        let child = {
            pair.slave
                .spawn_command(command)
                .map_err(|error| NativePtyError::external(error, "Failed to spawn shell"))?
        };
        drop(pair.slave);

        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| NativePtyError::external(error, "Failed to clone reader"))?;
        let writer = master
            .take_writer()
            .map_err(|error| NativePtyError::external(error, "Failed to take writer"))?;

        let killer = child.clone_killer();
        Ok(SpawnedNativePty {
            runtime: NativePtyRuntime::new(writer, killer, Box::new(PortablePtyResizer { master })),
            output: NativePtyOutput { reader, child },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePtyError {
    pub(crate) kind: std::io::ErrorKind,
    pub(crate) message: String,
}
impl NativePtyError {
    fn io(error: std::io::Error, context: &str) -> Self {
        Self {
            kind: error.kind(),
            message: format!("{context}: {error}"),
        }
    }
    fn external(error: impl AsRef<dyn std::error::Error + Send + Sync>, context: &str) -> Self {
        let error = error.as_ref();
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
        let mut kind = std::io::ErrorKind::Other;
        while let Some(current) = source {
            if let Some(error) = current.downcast_ref::<std::io::Error>() {
                kind = error.kind();
                break;
            }
            source = current.source();
        }
        Self {
            kind,
            message: format!("{context}: {error}"),
        }
    }
}
impl std::fmt::Display for NativePtyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
#[path = "native_pty_test.rs"]
mod native_pty_tests;
