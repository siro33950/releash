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
pub(crate) struct NativePtyRuntime {
    input: mpsc::SyncSender<Vec<u8>>,
    input_error: Arc<Mutex<Option<String>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn NativePtyResizer + Send>>>,
}

impl NativePtyRuntime {
    pub(crate) fn write(&self, data: &[u8]) -> Result<(), String> {
        if let Some(error) = self.input_error.lock().as_ref() {
            return Err(error.clone());
        }
        self.input
            .try_send(data.to_vec())
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "PTY input queue is full".to_string(),
                mpsc::TrySendError::Disconnected(_) => self
                    .input_error
                    .lock()
                    .clone()
                    .unwrap_or_else(|| "PTY input writer is unavailable".to_string()),
            })
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        self.resizer.lock().resize(rows, cols)
    }

    pub(crate) fn kill(&self) -> Result<(), String> {
        self.killer
            .lock()
            .kill()
            .map_err(|error| format!("Failed to kill PTY: {error}"))
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
                    .map_err(|error| format!("Failed to write to PTY: {error}"))
                    .and_then(|()| {
                        writer
                            .flush()
                            .map_err(|error| format!("Failed to flush PTY: {error}"))
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

    #[cfg(test)]
    pub(crate) fn from_parts(
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

pub(crate) trait NativePtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String>;
}

struct PortablePtyResizer {
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl NativePtyResizer for PortablePtyResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Failed to resize PTY: {error}"))
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
    pub(crate) fn spawn(&self, config: NativePtySpawnConfig) -> Result<SpawnedNativePty, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Failed to open PTY: {error}"))?;

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

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Failed to spawn shell: {error}"))?;
        drop(pair.slave);

        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| format!("Failed to clone reader: {error}"))?;
        let writer = master
            .take_writer()
            .map_err(|error| format!("Failed to take writer: {error}"))?;

        let killer = child.clone_killer();
        Ok(SpawnedNativePty {
            runtime: NativePtyRuntime::new(writer, killer, Box::new(PortablePtyResizer { master })),
            output: NativePtyOutput { reader, child },
        })
    }
}

#[cfg(test)]
#[path = "native_pty_test.rs"]
mod native_pty_tests;
