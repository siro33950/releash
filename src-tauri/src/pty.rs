use parking_lot::Mutex;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::{PtyExitMsg, PtyOutputMsg, WsMessage};
use crate::shell_integration;
use crate::ws_bridge::WsBroadcaster;

const OUTPUT_BUFFER_CAPACITY: usize = 64 * 1024;

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

struct PtySession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child_killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
    worktree_path: Option<String>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<u64, PtySession>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub worktree_path: Option<String>,
}

impl PtyManager {
    pub fn write(&self, pty_id: u64, data: &str) -> Result<(), String> {
        let writer = {
            let sessions = self.sessions.lock();
            let session = sessions
                .get(&pty_id)
                .ok_or_else(|| format!("PTY {} not found", pty_id))?;
            Arc::clone(&session.writer)
        };
        let mut writer = writer.lock();
        writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("Failed to write to PTY: {}", e))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush: {}", e))?;
        Ok(())
    }

    pub fn get_pty_size(&self, pty_id: u64) -> Result<(u16, u16), String> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(&pty_id)
            .ok_or_else(|| format!("PTY {} not found", pty_id))?;
        let master = session.master.lock();
        let size = master
            .get_size()
            .map_err(|e| format!("Failed to get PTY size: {}", e))?;
        Ok((size.cols, size.rows))
    }

    pub fn list_pty_sessions(&self) -> Vec<PtySessionInfo> {
        let sessions = self.sessions.lock();
        sessions
            .iter()
            .map(|(id, s)| PtySessionInfo {
                pty_id: *id,
                worktree_path: s.worktree_path.clone(),
            })
            .collect()
    }

    pub fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), String> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        let sessions = self.sessions.lock();
        let session = sessions
            .get(&pty_id)
            .ok_or_else(|| format!("PTY {} not found", pty_id))?;
        let master = session.master.lock();
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize PTY: {}", e))?;
        Ok(())
    }

    pub fn spawn(
        &self,
        app: &AppHandle,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        worktree_path: Option<String>,
    ) -> Result<u64, String> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        let integration_dir = app
            .path()
            .app_data_dir()
            .ok()
            .and_then(|d| shell_integration::create_shell_integration_files(&d).ok());

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let mut cmd = if let Some(ref int_dir) = integration_dir {
            if shell.ends_with("/bash") {
                let mut c = CommandBuilder::new(&shell);
                c.arg("--rcfile");
                c.arg(int_dir.join("bash-init.sh"));
                c
            } else if shell.ends_with("/zsh") {
                let mut c = CommandBuilder::new(&shell);
                let user_zdotdir = std::env::var("ZDOTDIR")
                    .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_default());
                c.env("RELEASH_USER_ZDOTDIR", user_zdotdir);
                c.env("ZDOTDIR", int_dir.join("zsh"));
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

        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {}", e))?;

        let child_killer = child.clone_killer();
        let mut child = child;

        let pty_id = generate_pty_id();

        let master = pair.master;
        let mut reader = master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone reader: {}", e))?;
        let writer = master
            .take_writer()
            .map_err(|e| format!("Failed to take writer: {}", e))?;

        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_CAPACITY)));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code_holder = Arc::new(Mutex::new(None::<i32>));

        let session = PtySession {
            master: Arc::new(Mutex::new(master)),
            writer: Arc::new(Mutex::new(writer)),
            child_killer: Arc::new(Mutex::new(child_killer)),
            worktree_path,
            output_buffer: Arc::clone(&output_buffer),
            exited: Arc::clone(&exited),
            exit_code: Arc::clone(&exit_code_holder),
        };

        self.sessions.lock().insert(pty_id, session);

        let app_clone = app.clone();
        let pty_id_clone = pty_id;
        let output_buf_clone = Arc::clone(&output_buffer);
        std::thread::spawn(move || {
            let ws = app_clone.try_state::<Arc<WsBroadcaster>>();
            let mut buf = [0u8; 4096];
            let mut pending = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);

                        let valid_up_to = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) => e.valid_up_to(),
                        };

                        if valid_up_to == 0 {
                            continue;
                        }

                        let raw = std::str::from_utf8(&pending[..valid_up_to])
                            .unwrap()
                            .to_string();
                        pending = pending[valid_up_to..].to_vec();

                        let result = shell_integration::strip_osc_cmd_done(&raw);

                        if !result.filtered_output.is_empty() {
                            {
                                let mut ring = output_buf_clone.lock();
                                let bytes = result.filtered_output.as_bytes();
                                if bytes.len() >= OUTPUT_BUFFER_CAPACITY {
                                    ring.clear();
                                    ring.extend(&bytes[bytes.len() - OUTPUT_BUFFER_CAPACITY..]);
                                } else {
                                    let overflow = (ring.len() + bytes.len())
                                        .saturating_sub(OUTPUT_BUFFER_CAPACITY);
                                    if overflow > 0 {
                                        ring.drain(..overflow);
                                    }
                                    ring.extend(bytes);
                                }
                            }

                            let _ = app_clone.emit(
                                "pty-output",
                                PtyOutput {
                                    pty_id: pty_id_clone,
                                    data: result.filtered_output.clone(),
                                },
                            );
                            if let Some(ws) = &ws {
                                ws.try_send(WsMessage::PtyOutput(PtyOutputMsg {
                                    pty_id: pty_id_clone,
                                    data: result.filtered_output,
                                }));
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let app_clone = app.clone();
        let pty_id_clone = pty_id;
        std::thread::spawn(move || {
            let exit_status = child.wait();
            let code = exit_status.ok().map(|s| s.exit_code() as i32);

            exited.store(true, Ordering::SeqCst);
            *exit_code_holder.lock() = code;

            let _ = app_clone.emit(
                "pty-exit",
                PtyExit {
                    pty_id: pty_id_clone,
                    exit_code: code,
                },
            );
            if let Some(ws) = app_clone.try_state::<Arc<WsBroadcaster>>() {
                ws.try_send(WsMessage::PtyExit(PtyExitMsg {
                    pty_id: pty_id_clone,
                    exit_code: code,
                }));
            }
        });

        Ok(pty_id)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtyOutput {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtyExit {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[tauri::command]
pub fn spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: Option<String>,
) -> Result<u64, String> {
    state.spawn(&app, rows, cols, cwd, worktree_path)
}

#[tauri::command]
pub fn write_pty(
    state: State<'_, Arc<PtyManager>>,
    pty_id: u64,
    data: String,
) -> Result<(), String> {
    state.write(pty_id, &data)
}

#[tauri::command]
pub fn resize_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    state.resize(pty_id, rows, cols)?;
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        ws.try_send(WsMessage::PtyResize(crate::protocol::PtyResize {
            pty_id,
            rows,
            cols,
        }));
    }
    Ok(())
}

#[tauri::command]
pub fn list_pty_sessions(state: State<'_, Arc<PtyManager>>) -> Vec<PtySessionInfo> {
    state.list_pty_sessions()
}

#[tauri::command]
pub fn kill_pty(state: State<'_, Arc<PtyManager>>, pty_id: u64) -> Result<(), String> {
    let mut sessions = state.sessions.lock();

    let session = sessions
        .get(&pty_id)
        .ok_or_else(|| format!("PTY {} not found", pty_id))?;

    // already exited → just remove session
    if !session.exited.load(Ordering::SeqCst) {
        session
            .child_killer
            .lock()
            .kill()
            .map_err(|e| format!("Failed to kill PTY {}: {}", pty_id, e))?;
    }

    sessions.remove(&pty_id);

    Ok(())
}

#[derive(Serialize)]
pub struct GetOrSpawnPtyResult {
    pty_id: u64,
    buffered_output: String,
    is_new: bool,
    is_exited: bool,
    exit_code: Option<i32>,
}

#[tauri::command]
pub fn get_or_spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: String,
) -> Result<GetOrSpawnPtyResult, String> {
    // Search for an existing session matching the worktree_path
    {
        let sessions = state.sessions.lock();
        for (&id, session) in sessions.iter() {
            if session.worktree_path.as_deref() == Some(&worktree_path) {
                let is_exited = session.exited.load(Ordering::SeqCst);
                let exit_code = *session.exit_code.lock();

                // Snapshot the output buffer
                let ring = session.output_buffer.lock();
                let (a, b) = ring.as_slices();
                let mut bytes = Vec::with_capacity(a.len() + b.len());
                bytes.extend_from_slice(a);
                bytes.extend_from_slice(b);
                let buffered_output = String::from_utf8_lossy(&bytes).into_owned();

                return Ok(GetOrSpawnPtyResult {
                    pty_id: id,
                    buffered_output,
                    is_new: false,
                    is_exited,
                    exit_code,
                });
            }
        }
    }

    // No existing session — spawn a new one
    let pty_id = state.spawn(&app, rows, cols, cwd, Some(worktree_path))?;
    Ok(GetOrSpawnPtyResult {
        pty_id,
        buffered_output: String::new(),
        is_new: true,
        is_exited: false,
        exit_code: None,
    })
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    state: State<'_, Arc<PtyManager>>,
    worktree_path: String,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock();
    let ids_to_remove: Vec<u64> = sessions
        .iter()
        .filter(|(_, s)| s.worktree_path.as_deref() == Some(&worktree_path))
        .map(|(&id, _)| id)
        .collect();

    for id in ids_to_remove {
        if let Some(session) = sessions.get(&id) {
            if !session.exited.load(Ordering::SeqCst) {
                let _ = session.child_killer.lock().kill();
            }
        }
        sessions.remove(&id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pty_id_is_monotonically_increasing() {
        let id1 = generate_pty_id();
        let id2 = generate_pty_id();
        let id3 = generate_pty_id();
        assert!(id2 > id1);
        assert!(id3 > id2);
    }

    #[test]
    fn test_generate_pty_id_uniqueness() {
        let ids: Vec<u64> = (0..100).map(|_| generate_pty_id()).collect();
        let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len());
    }

    #[test]
    fn test_pty_manager_default() {
        let pm = PtyManager::default();
        assert!(pm.sessions.lock().is_empty());
    }

    #[test]
    fn test_list_pty_sessions_empty() {
        let pm = PtyManager::default();
        let sessions = pm.list_pty_sessions();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_write_nonexistent_pty_returns_error() {
        let pm = PtyManager::default();
        let result = pm.write(99999, "hello");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_resize_nonexistent_pty_returns_error() {
        let pm = PtyManager::default();
        let result = pm.resize(99999, 24, 80);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_get_pty_size_nonexistent_returns_error() {
        let pm = PtyManager::default();
        let result = pm.get_pty_size(99999);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_output_buffer_capacity_value() {
        assert_eq!(OUTPUT_BUFFER_CAPACITY, 64 * 1024);
    }

    #[test]
    fn test_resize_zero_rows_returns_ok() {
        let pm = PtyManager::default();
        let result = pm.resize(99999, 0, 80);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resize_zero_cols_returns_ok() {
        let pm = PtyManager::default();
        let result = pm.resize(99999, 24, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resize_zero_both_returns_ok() {
        let pm = PtyManager::default();
        let result = pm.resize(99999, 0, 0);
        assert!(result.is_ok());
    }
}
