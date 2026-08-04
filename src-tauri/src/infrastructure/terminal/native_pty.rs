use std::io::{Read, Write};
use std::sync::Arc;

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
    pub(crate) exec_command: Option<String>,
}

pub(crate) struct SpawnedNativePty {
    pub(crate) runtime: NativePtyRuntime,
    pub(crate) output: NativePtyOutput,
}

#[derive(Clone)]
pub(crate) struct NativePtyRuntime {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn NativePtyResizer + Send>>>,
}

impl NativePtyRuntime {
    pub(crate) fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut writer = self.writer.lock();
        writer
            .write_all(data)
            .map_err(|error| format!("Failed to write to PTY: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("Failed to flush PTY: {error}"))
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

    #[cfg(test)]
    pub(crate) fn from_parts(
        writer: Box<dyn Write + Send>,
        killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
        resizer: Box<dyn NativePtyResizer + Send>,
    ) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
            killer: Arc::new(Mutex::new(killer)),
            resizer: Arc::new(Mutex::new(resizer)),
        }
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

        let mut command = if let Some(exec) = config.exec_command {
            let mut command = CommandBuilder::new(&config.shell);
            command.arg("-l");
            command.arg("-c");
            command.arg(exec);
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

        #[cfg(not(target_os = "windows"))]
        {
            command.env("TERM", "xterm-256color");
            command.env("COLORTERM", "truecolor");
            if std::env::var("LANG").is_err() {
                command.env("LANG", "en_US.UTF-8");
            }
        }

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
            runtime: NativePtyRuntime {
                writer: Arc::new(Mutex::new(writer)),
                killer: Arc::new(Mutex::new(killer)),
                resizer: Arc::new(Mutex::new(Box::new(PortablePtyResizer { master }))),
            },
            output: NativePtyOutput { reader, child },
        })
    }
}

#[cfg(test)]
#[path = "native_pty_test.rs"]
mod native_pty_tests;
