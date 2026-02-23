pub mod backend;
mod direct;
mod lifecycle;
mod tmux;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::{PtyExitMsg, PtyOutputMsg, WsMessage};
use crate::shell_integration;
use crate::ws_bridge::WsBroadcaster;

use backend::{PtyBackend, PtyResizer, SpawnConfig};
use direct::DirectPtyBackend;
use tmux::TmuxPtyBackend;

const OUTPUT_BUFFER_CAPACITY: usize = 64 * 1024;
const MAX_PENDING_BYTES: usize = 16 * 1024;

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

struct PtySession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn PtyResizer + Send>>>,
    worktree_path: Option<String>,
    label: Option<String>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
    is_restored: bool,
}

pub struct PtyManager {
    sessions: Mutex<HashMap<u64, PtySession>>,
    backend: Box<dyn PtyBackend>,
    _lifecycle: Mutex<lifecycle::SessionLifecycle>,
}

impl Default for PtyManager {
    fn default() -> Self {
        let backend: Box<dyn PtyBackend> = if TmuxPtyBackend::is_available() {
            log::info!("Using tmux PTY backend");
            Box::new(TmuxPtyBackend::new())
        } else {
            log::info!("Using direct PTY backend (tmux not available)");
            Box::new(DirectPtyBackend::new())
        };

        Self {
            sessions: Mutex::new(HashMap::new()),
            backend,
            _lifecycle: Mutex::new(lifecycle::SessionLifecycle::new()),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn spawn_output_reader(
    app: AppHandle,
    pty_id: u64,
    mut reader: Box<dyn Read + Send>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    exited: Arc<AtomicBool>,
    exit_code_holder: Arc<Mutex<Option<i32>>>,
) {
    std::thread::spawn(move || {
        let ws = app.try_state::<Arc<WsBroadcaster>>();
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
                        if pending.len() > MAX_PENDING_BYTES {
                            log::warn!(
                                "PTY {}: dropping {} bytes of invalid UTF-8",
                                pty_id,
                                pending.len()
                            );
                            pending.clear();
                        }
                        continue;
                    }

                    let raw = std::str::from_utf8(&pending[..valid_up_to])
                        .unwrap()
                        .to_string();
                    pending = pending[valid_up_to..].to_vec();

                    let result = shell_integration::strip_osc_cmd_done(&raw);

                    if !result.filtered_output.is_empty() {
                        {
                            let mut ring = output_buffer.lock();
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

                        let _ = app.emit(
                            "pty-output",
                            PtyOutput {
                                pty_id,
                                data: result.filtered_output.clone(),
                            },
                        );
                        if let Some(ws) = &ws {
                            ws.try_send(WsMessage::PtyOutput(PtyOutputMsg {
                                pty_id,
                                data: result.filtered_output,
                            }));
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        exited.store(true, Ordering::SeqCst);
        *exit_code_holder.lock() = None;

        let _ = app.emit(
            "pty-exit",
            PtyExit {
                pty_id,
                exit_code: None,
            },
        );
        if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
            ws.try_send(WsMessage::PtyExit(PtyExitMsg {
                pty_id,
                exit_code: None,
            }));
        }
    });
}

impl PtyManager {
    #[allow(dead_code)]
    pub fn with_backend(backend: Box<dyn PtyBackend>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backend,
            _lifecycle: Mutex::new(lifecycle::SessionLifecycle::new()),
        }
    }

    #[allow(dead_code)]
    pub fn backend_name(&self) -> &'static str {
        self.backend.backend_name()
    }

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
        let resizer = session.resizer.lock();
        resizer.get_size()
    }

    pub fn list_pty_sessions(&self) -> Vec<PtySessionInfo> {
        let sessions = self.sessions.lock();
        sessions
            .iter()
            .map(|(id, s)| PtySessionInfo {
                pty_id: *id,
                worktree_path: s.worktree_path.clone(),
                label: s.label.clone(),
            })
            .collect()
    }

    pub fn kill(&self, pty_id: u64) -> Result<(), String> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get(&pty_id)
            .ok_or_else(|| format!("PTY {} not found", pty_id))?;
        if !session.exited.load(Ordering::SeqCst) {
            session
                .killer
                .lock()
                .kill()
                .map_err(|e| format!("Failed to kill PTY {}: {}", pty_id, e))?;
        }
        sessions.remove(&pty_id);
        Ok(())
    }

    pub fn resize(&self, pty_id: u64, rows: u16, cols: u16) -> Result<(), String> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        let sessions = self.sessions.lock();
        let session = sessions
            .get(&pty_id)
            .ok_or_else(|| format!("PTY {} not found", pty_id))?;
        let mut resizer = session.resizer.lock();
        resizer.resize(rows, cols)
    }

    pub fn spawn(
        &self,
        app: &AppHandle,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        worktree_path: Option<String>,
        label: Option<String>,
    ) -> Result<u64, String> {
        let integration_dir = app
            .path()
            .app_data_dir()
            .ok()
            .and_then(|d| shell_integration::create_shell_integration_files(&d).ok());

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let config = SpawnConfig {
            rows,
            cols,
            cwd,
            worktree_path: worktree_path.clone(),
            label: label.clone(),
            shell,
            integration_dir,
        };

        let backend_session = self.backend.spawn(config)?;

        let pty_id = generate_pty_id();
        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_CAPACITY)));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code_holder = Arc::new(Mutex::new(None::<i32>));

        let writer = backend_session.writer;
        let killer = backend_session.killer;
        let resizer = backend_session.resizer;
        let reader = backend_session.reader;

        let session = PtySession {
            writer,
            killer: Arc::new(Mutex::new(killer.lock().clone_killer())),
            resizer,
            worktree_path,
            label,
            output_buffer: Arc::clone(&output_buffer),
            exited: Arc::clone(&exited),
            exit_code: Arc::clone(&exit_code_holder),
            is_restored: false,
        };

        self.sessions.lock().insert(pty_id, session);

        spawn_output_reader(
            app.clone(),
            pty_id,
            reader,
            output_buffer,
            exited,
            exit_code_holder,
        );

        Ok(pty_id)
    }

    #[allow(dead_code)]
    pub fn restore_sessions(&self, app: &AppHandle) -> Vec<(u64, PtySessionInfo)> {
        let existing = match self.backend.list_existing() {
            Ok(sessions) => sessions,
            Err(e) => {
                log::warn!("Failed to list existing sessions: {}", e);
                return vec![];
            }
        };

        let mut restored = vec![];

        for existing_session in existing {
            match self.backend.attach(&existing_session.session_id) {
                Ok(backend_session) => {
                    let pty_id = generate_pty_id();
                    let output_buffer =
                        Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_CAPACITY)));
                    let exited = Arc::new(AtomicBool::new(false));
                    let exit_code_holder = Arc::new(Mutex::new(None::<i32>));

                    let writer = backend_session.writer;
                    let killer = backend_session.killer;
                    let resizer = backend_session.resizer;
                    let reader = backend_session.reader;

                    let worktree_path = existing_session.worktree_path.clone();
                    let label = existing_session.label.clone();

                    let session = PtySession {
                        writer,
                        killer: Arc::new(Mutex::new(killer.lock().clone_killer())),
                        resizer,
                        worktree_path: worktree_path.clone(),
                        label: label.clone(),
                        output_buffer: Arc::clone(&output_buffer),
                        exited: Arc::clone(&exited),
                        exit_code: Arc::clone(&exit_code_holder),
                        is_restored: true,
                    };

                    self.sessions.lock().insert(pty_id, session);

                    let info = PtySessionInfo {
                        pty_id,
                        worktree_path,
                        label,
                    };
                    restored.push((pty_id, info.clone()));

                    spawn_output_reader(
                        app.clone(),
                        pty_id,
                        reader,
                        output_buffer,
                        exited,
                        exit_code_holder,
                    );

                    log::info!(
                        "Restored tmux session '{}' as pty_id={}",
                        existing_session.session_id,
                        pty_id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "Failed to attach to session '{}': {}",
                        existing_session.session_id,
                        e
                    );
                }
            }
        }

        restored
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
    label: Option<String>,
) -> Result<u64, String> {
    state.spawn(&app, rows, cols, cwd, worktree_path, label)
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
    state.kill(pty_id)
}

#[derive(Serialize)]
pub struct GetOrSpawnPtyResult {
    pty_id: u64,
    buffered_output: String,
    is_new: bool,
    is_exited: bool,
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    is_restored: bool,
}

#[tauri::command]
pub fn get_or_spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: String,
    label: Option<String>,
) -> Result<GetOrSpawnPtyResult, String> {
    // Search for an existing session matching the (worktree_path, label) pair
    {
        let sessions = state.sessions.lock();
        for (&id, session) in sessions.iter() {
            if session.worktree_path.as_deref() == Some(&worktree_path) && session.label == label {
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
                    label: session.label.clone(),
                    is_restored: session.is_restored,
                });
            }
        }
    }

    // No existing session — spawn a new one
    let pty_id = state.spawn(&app, rows, cols, cwd, Some(worktree_path), label.clone())?;
    Ok(GetOrSpawnPtyResult {
        pty_id,
        buffered_output: String::new(),
        is_new: true,
        is_exited: false,
        exit_code: None,
        label,
        is_restored: false,
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
                let _ = session.killer.lock().kill();
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

    #[test]
    fn test_pty_session_info_serialization_with_label() {
        let info = PtySessionInfo {
            pty_id: 1,
            worktree_path: Some("/repo".to_string()),
            label: Some("dev".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"label\":\"dev\""));
        let deserialized: PtySessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.label.unwrap(), "dev");
    }

    #[test]
    fn test_pty_session_info_serialization_without_label() {
        let info = PtySessionInfo {
            pty_id: 1,
            worktree_path: Some("/repo".to_string()),
            label: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("\"label\""));
    }

    #[test]
    fn test_kill_nonexistent_pty_returns_error() {
        let pm = PtyManager::default();
        let result = pm.kill(99999);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_backend_name() {
        let pm = PtyManager::default();
        let name = pm.backend_name();
        assert!(name == "direct" || name == "tmux");
    }

    #[test]
    fn test_with_backend_direct() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        assert_eq!(pm.backend_name(), "direct");
    }

    #[test]
    fn test_get_or_spawn_result_serialization() {
        let result = GetOrSpawnPtyResult {
            pty_id: 1,
            buffered_output: String::new(),
            is_new: true,
            is_exited: false,
            exit_code: None,
            label: None,
            is_restored: false,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"is_restored\":false"));
        assert!(!json.contains("\"label\""));
    }

    #[test]
    fn test_get_or_spawn_result_serialization_restored() {
        let result = GetOrSpawnPtyResult {
            pty_id: 1,
            buffered_output: "hello".to_string(),
            is_new: false,
            is_exited: false,
            exit_code: None,
            label: Some("dev".to_string()),
            is_restored: true,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"is_restored\":true"));
        assert!(json.contains("\"label\":\"dev\""));
    }
}
