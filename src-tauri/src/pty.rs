use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone)]
struct PtySession {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<u64, PtySession>>,
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
    state: State<'_, PtyManager>,
    rows: u16,
    cols: u16,
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

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let cmd = CommandBuilder::new(shell);

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Failed to spawn shell: {}", e))?;

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
    };

    state.sessions.lock().insert(pty_id, session);

    // Spawn thread to read PTY output
    let app_clone = app.clone();
    let pty_id_clone = pty_id;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_clone.emit(
                        "pty-output",
                        PtyOutput {
                            pty_id: pty_id_clone,
                            data,
                        },
                    );
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

        // Remove session from manager
        if let Some(manager) = app_clone.try_state::<PtyManager>() {
            manager.sessions.lock().remove(&pty_id_clone);
        }
    });

    Ok(pty_id)
}

#[tauri::command]
pub fn write_pty(state: State<'_, PtyManager>, pty_id: u64, data: String) -> Result<(), String> {
    let sessions = state.sessions.lock();
    let session = sessions
        .get(&pty_id)
        .ok_or_else(|| format!("PTY {} not found", pty_id))?;

    let mut writer = session.writer.lock();
    writer
        .write_all(data.as_bytes())
        .map_err(|e| format!("Failed to write to PTY: {}", e))?;
    writer.flush().map_err(|e| format!("Failed to flush: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn resize_pty(
    state: State<'_, PtyManager>,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let sessions = state.sessions.lock();
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

#[tauri::command]
pub fn kill_pty(state: State<'_, PtyManager>, pty_id: u64) -> Result<(), String> {
    let mut sessions = state.sessions.lock();
    sessions
        .remove(&pty_id)
        .ok_or_else(|| format!("PTY {} not found", pty_id))?;
    Ok(())
}
