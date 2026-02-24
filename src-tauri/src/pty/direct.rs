use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::sync::Arc;

use super::backend::{BackendSession, PtyBackend, PtyResizer, SpawnConfig};

struct DirectResizer {
    master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PtyResizer for DirectResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize PTY: {}", e))
    }

    fn get_size(&self) -> Result<(u16, u16), String> {
        let size = self
            .master
            .get_size()
            .map_err(|e| format!("Failed to get PTY size: {}", e))?;
        Ok((size.cols, size.rows))
    }
}

pub struct DirectPtyBackend;

impl DirectPtyBackend {
    pub fn new() -> Self {
        Self
    }
}

impl PtyBackend for DirectPtyBackend {
    fn spawn(&self, config: SpawnConfig) -> Result<BackendSession, String> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        let shell = &config.shell;
        let mut cmd = if let Some(ref int_dir) = config.integration_dir {
            if shell.ends_with("/bash") {
                let mut c = CommandBuilder::new(shell);
                c.arg("--rcfile");
                c.arg(int_dir.join("bash-init.sh"));
                c
            } else if shell.ends_with("/zsh") {
                let mut c = CommandBuilder::new(shell);
                let user_zdotdir = std::env::var("ZDOTDIR")
                    .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
                c.env("RELEASH_USER_ZDOTDIR", user_zdotdir);
                c.env("ZDOTDIR", int_dir.join("zsh"));
                c
            } else if shell.ends_with("/fish") {
                let mut c = CommandBuilder::new(shell);
                c.arg("-C");
                c.arg(format!(
                    "source '{}'",
                    int_dir.join("fish-init.fish").display()
                ));
                c
            } else {
                CommandBuilder::new_default_prog()
            }
        } else {
            CommandBuilder::new_default_prog()
        };

        #[cfg(not(target_os = "windows"))]
        {
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
            if std::env::var("LANG").is_err() {
                cmd.env("LANG", "en_US.UTF-8");
            }
        }

        if let Some(dir) = config.cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {}", e))?;
        drop(pair.slave);

        let child_killer = child.clone_killer();

        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone reader: {}", e))?;
        let writer = master
            .take_writer()
            .map_err(|e| format!("Failed to take writer: {}", e))?;

        let resizer = DirectResizer { master };

        Ok(BackendSession {
            reader,
            writer: Arc::new(Mutex::new(writer)),
            killer: Arc::new(Mutex::new(child_killer)),
            resizer: Arc::new(Mutex::new(Box::new(resizer))),
        })
    }

    fn backend_name(&self) -> &'static str {
        "direct"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_name() {
        let backend = DirectPtyBackend::new();
        assert_eq!(backend.backend_name(), "direct");
    }

    #[test]
    fn test_new_creates_instance() {
        let _backend = DirectPtyBackend::new();
    }
}
