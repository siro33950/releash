use parking_lot::Mutex;
use portable_pty::ChildKiller;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::backend::{BackendSession, ExistingSession, PtyBackend, PtyResizer, SpawnConfig};

const SESSION_PREFIX: &str = "releash";

fn hash_string(s: &str) -> String {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn session_name(worktree_path: Option<&str>, label: Option<&str>) -> String {
    let wt_hash = worktree_path
        .map(hash_string)
        .unwrap_or_else(|| "none".to_string());
    let label_hash = label
        .map(hash_string)
        .unwrap_or_else(|| "default".to_string());
    format!("{}-{}-{}", SESSION_PREFIX, wt_hash, label_hash)
}

struct TmuxResizer {
    session: String,
}

impl PtyResizer for TmuxResizer {
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let output = Command::new("tmux")
            .args([
                "resize-window",
                "-t",
                &self.session,
                "-x",
                &cols.to_string(),
                "-y",
                &rows.to_string(),
            ])
            .output()
            .map_err(|e| format!("Failed to resize tmux session: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("tmux resize failed: {}", stderr));
        }
        Ok(())
    }

    fn get_size(&self) -> Result<(u16, u16), String> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-t",
                &self.session,
                "-p",
                "#{window_width} #{window_height}",
            ])
            .output()
            .map_err(|e| format!("Failed to get tmux size: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("tmux display-message failed: {}", stderr));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(format!("Unexpected tmux size output: {}", stdout));
        }
        let cols: u16 = parts[0]
            .parse()
            .map_err(|_| format!("Invalid cols: {}", parts[0]))?;
        let rows: u16 = parts[1]
            .parse()
            .map_err(|_| format!("Invalid rows: {}", parts[1]))?;
        Ok((cols, rows))
    }
}

#[derive(Debug)]
struct TmuxKiller {
    session: String,
    killed: AtomicBool,
}

impl ChildKiller for TmuxKiller {
    fn kill(&mut self) -> Result<(), io::Error> {
        if self.killed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let output = Command::new("tmux")
            .args(["kill-session", "-t", &self.session])
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::other(format!(
                "tmux kill-session failed: {}",
                stderr
            )));
        }
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(TmuxKiller {
            session: self.session.clone(),
            killed: AtomicBool::new(self.killed.load(Ordering::SeqCst)),
        })
    }
}

struct TmuxPipeReader {
    child_stdout: std::process::ChildStdout,
}

impl Read for TmuxPipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.child_stdout.read(buf)
    }
}

struct TmuxWriter {
    session: String,
}

impl Write for TmuxWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let data = String::from_utf8_lossy(buf);
        let output = Command::new("tmux")
            .args(["send-keys", "-t", &self.session, "-l", &data])
            .output()
            .map_err(io::Error::other)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::other(stderr.to_string()));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct TmuxPtyBackend;

impl TmuxPtyBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn is_available() -> bool {
        Command::new("tmux")
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn create_session(
        &self,
        name: &str,
        config: &SpawnConfig,
    ) -> Result<std::process::Child, String> {
        let shell = &config.shell;

        let mut args = vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-s".to_string(),
            name.to_string(),
            "-x".to_string(),
            config.cols.to_string(),
            "-y".to_string(),
            config.rows.to_string(),
        ];

        if let Some(ref cwd) = config.cwd {
            args.push("-c".to_string());
            args.push(cwd.clone());
        }

        args.push(shell.clone());

        let session_output = Command::new("tmux")
            .args(&args)
            .output()
            .map_err(|e| format!("Failed to create tmux session: {}", e))?;
        if !session_output.status.success() {
            let stderr = String::from_utf8_lossy(&session_output.stderr);
            return Err(format!("tmux new-session failed: {}", stderr));
        }

        // Set up environment in the tmux session
        #[cfg(not(target_os = "windows"))]
        {
            let _ = Command::new("tmux")
                .args(["set-environment", "-t", name, "TERM", "xterm-256color"])
                .output();
            let _ = Command::new("tmux")
                .args(["set-environment", "-t", name, "COLORTERM", "truecolor"])
                .output();
        }

        // Use `tmux pipe-pane` + `cat` to capture output via a subprocess
        let pipe_child = Command::new("tmux")
            .args(["pipe-pane", "-t", name, "-o", "cat"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start tmux pipe-pane: {}", e))?;

        Ok(pipe_child)
    }

    #[allow(dead_code)]
    fn attach_session(&self, name: &str) -> Result<std::process::Child, String> {
        let pipe_child = Command::new("tmux")
            .args(["pipe-pane", "-t", name, "-o", "cat"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to attach tmux pipe-pane: {}", e))?;

        Ok(pipe_child)
    }
}

impl PtyBackend for TmuxPtyBackend {
    fn spawn(&self, config: SpawnConfig) -> Result<BackendSession, String> {
        let name = session_name(config.worktree_path.as_deref(), config.label.as_deref());

        let pipe_child = self.create_session(&name, &config)?;
        let reader = pipe_child
            .stdout
            .ok_or_else(|| "Failed to get pipe-pane stdout".to_string())?;

        Ok(BackendSession {
            reader: Box::new(TmuxPipeReader {
                child_stdout: reader,
            }),
            writer: Arc::new(Mutex::new(Box::new(TmuxWriter {
                session: name.clone(),
            }))),
            killer: Arc::new(Mutex::new(Box::new(TmuxKiller {
                session: name.clone(),
                killed: AtomicBool::new(false),
            }))),
            resizer: Arc::new(Mutex::new(Box::new(TmuxResizer { session: name }))),
        })
    }

    fn attach(&self, session_id: &str) -> Result<BackendSession, String> {
        let pipe_child = self.attach_session(session_id)?;
        let reader = pipe_child
            .stdout
            .ok_or_else(|| "Failed to get pipe-pane stdout".to_string())?;

        Ok(BackendSession {
            reader: Box::new(TmuxPipeReader {
                child_stdout: reader,
            }),
            writer: Arc::new(Mutex::new(Box::new(TmuxWriter {
                session: session_id.to_string(),
            }))),
            killer: Arc::new(Mutex::new(Box::new(TmuxKiller {
                session: session_id.to_string(),
                killed: AtomicBool::new(false),
            }))),
            resizer: Arc::new(Mutex::new(Box::new(TmuxResizer {
                session: session_id.to_string(),
            }))),
        })
    }

    fn list_existing(&self) -> Result<Vec<ExistingSession>, String> {
        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
            .map_err(|e| format!("Failed to list tmux sessions: {}", e))?;

        if !output.status.success() {
            // No server running = no sessions
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let sessions: Vec<ExistingSession> = stdout
            .lines()
            .filter(|line| line.starts_with(SESSION_PREFIX))
            .map(|line| ExistingSession {
                session_id: line.to_string(),
                worktree_path: None,
                label: None,
            })
            .collect();

        Ok(sessions)
    }

    fn backend_name(&self) -> &'static str {
        "tmux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_name_generation() {
        let name = session_name(Some("/repo"), Some("dev"));
        assert!(name.starts_with("releash-"));
        assert!(!name.contains('/'));
    }

    #[test]
    fn test_session_name_without_worktree() {
        let name = session_name(None, Some("dev"));
        assert!(name.starts_with("releash-none-"));
    }

    #[test]
    fn test_session_name_without_label() {
        let name = session_name(Some("/repo"), None);
        assert!(name.ends_with("-default"));
    }

    #[test]
    fn test_session_name_uniqueness() {
        let name1 = session_name(Some("/repo1"), Some("dev"));
        let name2 = session_name(Some("/repo2"), Some("dev"));
        assert_ne!(name1, name2);
    }

    #[test]
    fn test_session_name_deterministic() {
        let name1 = session_name(Some("/repo"), Some("dev"));
        let name2 = session_name(Some("/repo"), Some("dev"));
        assert_eq!(name1, name2);
    }

    #[test]
    fn test_hash_string() {
        let h1 = hash_string("test");
        let h2 = hash_string("test");
        assert_eq!(h1, h2);
        let h3 = hash_string("other");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_is_available_returns_bool() {
        // Just verify it doesn't panic
        let _ = TmuxPtyBackend::is_available();
    }

    #[test]
    fn test_backend_name() {
        let backend = TmuxPtyBackend::new();
        assert_eq!(backend.backend_name(), "tmux");
    }
}
