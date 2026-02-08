use parking_lot::Mutex;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::{PtyExitMsg, PtyOutputMsg, WsMessage};
use crate::shell_integration;
use crate::ws_bridge::WsBroadcaster;

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

struct PtySession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child_killer: Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>,
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<u64, PtySession>>,
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

    pub fn active_pty_id(&self) -> Option<u64> {
        let sessions = self.sessions.lock();
        sessions.keys().next().copied()
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

    pub fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), String> {
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

    let session = PtySession {
        master: Arc::new(Mutex::new(master)),
        writer: Arc::new(Mutex::new(writer)),
        child_killer: Arc::new(Mutex::new(child_killer)),
    };

    state.sessions.lock().insert(pty_id, session);

    // Spawn thread to read PTY output
    let app_clone = app.clone();
    let pty_id_clone = pty_id;
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
                Err(_) => break,
            }
        }
    });

    // Spawn thread to wait for process exit
    let app_clone = app.clone();
    let pty_id_clone = pty_id;
    std::thread::spawn(move || {
        let exit_status = child.wait();
        let exit_code = exit_status.ok().map(|s| s.exit_code() as i32);
        let _ = app_clone.emit(
            "pty-exit",
            PtyExit {
                pty_id: pty_id_clone,
                exit_code,
            },
        );
        if let Some(ws) = app_clone.try_state::<Arc<WsBroadcaster>>() {
            ws.try_send(WsMessage::PtyExit(PtyExitMsg {
                pty_id: pty_id_clone,
                exit_code,
            }));
        }

        // Remove session from manager
        if let Some(manager) = app_clone.try_state::<PtyManager>() {
            manager.sessions.lock().remove(&pty_id_clone);
        }
    });

    Ok(pty_id)
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
pub fn kill_pty(state: State<'_, Arc<PtyManager>>, pty_id: u64) -> Result<(), String> {
    let mut sessions = state.sessions.lock();

    // 先にセッションを取得（削除はまだしない）
    let session = sessions
        .get(&pty_id)
        .ok_or_else(|| format!("PTY {} not found", pty_id))?;

    // kill()を先に実行し、エラーがあれば返す
    session
        .child_killer
        .lock()
        .kill()
        .map_err(|e| format!("Failed to kill PTY {}: {}", pty_id, e))?;

    // kill()成功後にセッション削除
    sessions.remove(&pty_id);

    Ok(())
}
