pub mod backend;
mod direct;
pub mod oneshot;

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

const OUTPUT_BUFFER_CAPACITY: usize = 64 * 1024;
const MAX_PENDING_BYTES: usize = 16 * 1024;

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyKind {
    Agent,
    Terminal,
    OneShot,
}

struct PtySession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    killer: Arc<Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    resizer: Arc<Mutex<Box<dyn PtyResizer + Send>>>,
    session_key: String,
    worktree_path: Option<String>,
    label: Option<String>,
    kind: PtyKind,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
}

pub struct FoundSession {
    pub pty_id: u64,
    pub session_key: String,
    pub buffered_output: String,
    pub is_exited: bool,
    pub exit_code: Option<i32>,
    pub label: Option<String>,
    pub kind: PtyKind,
}

pub struct PtyManager {
    sessions: Mutex<HashMap<u64, PtySession>>,
    backend: Box<dyn PtyBackend>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backend: Box::new(DirectPtyBackend::new()),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub kind: PtyKind,
}

/// UTF-8 処理 + リングバッファ更新の純粋ロジック。
/// 戻り値: フィルタ済み出力文字列 (空ならイベント不要)
fn process_pty_output(
    raw_chunk: &[u8],
    pending: &mut Vec<u8>,
    output_buffer: &Mutex<VecDeque<u8>>,
    pty_id: u64,
) -> Option<String> {
    pending.extend_from_slice(raw_chunk);

    let valid_up_to = match std::str::from_utf8(pending) {
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
        return None;
    }

    let raw = std::str::from_utf8(&pending[..valid_up_to])
        .unwrap()
        .to_string();
    *pending = pending[valid_up_to..].to_vec();

    let result = shell_integration::strip_osc_cmd_done(&raw);

    if result.filtered_output.is_empty() {
        return None;
    }

    {
        let mut ring = output_buffer.lock();
        let bytes = result.filtered_output.as_bytes();
        if bytes.len() >= OUTPUT_BUFFER_CAPACITY {
            ring.clear();
            ring.extend(&bytes[bytes.len() - OUTPUT_BUFFER_CAPACITY..]);
        } else {
            let overflow = (ring.len() + bytes.len()).saturating_sub(OUTPUT_BUFFER_CAPACITY);
            if overflow > 0 {
                ring.drain(..overflow);
            }
            ring.extend(bytes);
        }
    }

    Some(result.filtered_output)
}

fn spawn_output_reader(
    app: AppHandle,
    pty_id: u64,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
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
                    if let Some(filtered) =
                        process_pty_output(&buf[..n], &mut pending, &output_buffer, pty_id)
                    {
                        let _ = app.emit(
                            "pty-output",
                            PtyOutput {
                                pty_id,
                                data: filtered.clone(),
                            },
                        );
                        if let Some(ws) = &ws {
                            ws.try_send(WsMessage::PtyOutput(PtyOutputMsg {
                                pty_id,
                                data: filtered,
                            }));
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }

        let exit_code = child.wait().ok().map(|status| status.exit_code() as i32);

        exited.store(true, Ordering::SeqCst);
        *exit_code_holder.lock() = exit_code;

        let _ = app.emit("pty-exit", PtyExit { pty_id, exit_code });
        if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
            ws.try_send(WsMessage::PtyExit(PtyExitMsg { pty_id, exit_code }));
        }

        // Delayed cleanup: remove exited session after 5 minutes
        let app_cleanup = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(300));
            if let Some(mgr) = app_cleanup.try_state::<Arc<PtyManager>>() {
                mgr.remove_if_exited(pty_id);
            }
        });
    });
}

impl PtyManager {
    #[allow(dead_code)]
    pub fn with_backend(backend: Box<dyn PtyBackend>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            backend,
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

    /// Returns `(cols, rows)` — note the order differs from `resize(rows, cols)`.
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
                session_key: s.session_key.clone(),
                worktree_path: s.worktree_path.clone(),
                label: s.label.clone(),
                kind: s.kind,
            })
            .collect()
    }

    pub fn kill(&self, pty_id: u64) -> Result<(), String> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get(&pty_id)
            .ok_or_else(|| format!("PTY {} not found", pty_id))?;
        let kill_result = if !session.exited.load(Ordering::SeqCst) {
            session
                .killer
                .lock()
                .kill()
                .map_err(|e| format!("Failed to kill PTY {}: {}", pty_id, e))
        } else {
            Ok(())
        };
        sessions.remove(&pty_id);
        kill_result
    }

    pub fn remove_if_exited(&self, pty_id: u64) {
        let mut sessions = self.sessions.lock();
        if let Some(session) = sessions.get(&pty_id) {
            if session.exited.load(Ordering::SeqCst) {
                sessions.remove(&pty_id);
            }
        }
    }

    pub fn get_exit_status(&self, pty_id: u64) -> Option<(bool, Option<i32>)> {
        let sessions = self.sessions.lock();
        sessions
            .get(&pty_id)
            .map(|s| (s.exited.load(Ordering::SeqCst), *s.exit_code.lock()))
    }

    fn build_found_session(id: u64, session: &PtySession) -> FoundSession {
        let is_exited = session.exited.load(Ordering::SeqCst);
        let exit_code = *session.exit_code.lock();

        let ring = session.output_buffer.lock();
        let (a, b) = ring.as_slices();
        let mut bytes = Vec::with_capacity(a.len() + b.len());
        bytes.extend_from_slice(a);
        bytes.extend_from_slice(b);
        let buffered_output = String::from_utf8_lossy(&bytes).into_owned();

        FoundSession {
            pty_id: id,
            session_key: session.session_key.clone(),
            buffered_output,
            is_exited,
            exit_code,
            label: session.label.clone(),
            kind: session.kind,
        }
    }

    pub fn find_session(&self, session_key: &str) -> Option<FoundSession> {
        let sessions = self.sessions.lock();
        for (&id, session) in sessions.iter() {
            if session.session_key == session_key {
                return Some(Self::build_found_session(id, session));
            }
        }
        None
    }

    pub fn kill_by_worktree(&self, worktree_path: &str) -> Vec<u64> {
        let mut sessions = self.sessions.lock();
        let ids_to_kill: Vec<u64> = sessions
            .iter()
            .filter(|(_, s)| s.worktree_path.as_deref() == Some(worktree_path))
            .map(|(&id, _)| id)
            .collect();

        let mut killed_ids = Vec::with_capacity(ids_to_kill.len());
        for id in ids_to_kill {
            let should_remove = if let Some(session) = sessions.get(&id) {
                if session.exited.load(Ordering::SeqCst) {
                    true
                } else {
                    match session.killer.lock().kill() {
                        Ok(()) => true,
                        Err(e) => {
                            log::error!("Failed to kill PTY {}: {}", id, e);
                            false
                        }
                    }
                }
            } else {
                false
            };
            if should_remove {
                sessions.remove(&id);
                killed_ids.push(id);
            }
        }
        killed_ids
    }

    pub fn gc_by_worktree(
        &self,
        worktree_path: &str,
        keep_keys: &[String],
        kind_filter: Option<PtyKind>,
    ) -> Vec<u64> {
        let mut sessions = self.sessions.lock();
        let ids_to_kill: Vec<u64> = sessions
            .iter()
            .filter(|(_, s)| {
                s.worktree_path.as_deref() == Some(worktree_path)
                    && !keep_keys.contains(&s.session_key)
                    && kind_filter.as_ref().is_none_or(|k| s.kind == *k)
            })
            .map(|(&id, _)| id)
            .collect();

        let mut killed_ids = Vec::with_capacity(ids_to_kill.len());
        for id in ids_to_kill {
            let should_remove = if let Some(session) = sessions.get(&id) {
                if session.exited.load(Ordering::SeqCst) {
                    true
                } else {
                    match session.killer.lock().kill() {
                        Ok(()) => true,
                        Err(e) => {
                            log::error!("Failed to kill PTY {}: {}", id, e);
                            false
                        }
                    }
                }
            } else {
                false
            };
            if should_remove {
                sessions.remove(&id);
                killed_ids.push(id);
            }
        }
        killed_ids
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

    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        app: &AppHandle,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        worktree_path: Option<String>,
        label: Option<String>,
        kind: PtyKind,
    ) -> Result<(u64, String), String> {
        self.spawn_inner(app, rows, cols, cwd, worktree_path, label, None, kind)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_exec(
        &self,
        app: &AppHandle,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        worktree_path: Option<String>,
        label: Option<String>,
        exec_command: String,
        kind: PtyKind,
    ) -> Result<(u64, String), String> {
        self.spawn_inner(
            app,
            rows,
            cols,
            cwd,
            worktree_path,
            label,
            Some(exec_command),
            kind,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_inner(
        &self,
        app: &AppHandle,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        worktree_path: Option<String>,
        label: Option<String>,
        exec_command: Option<String>,
        kind: PtyKind,
    ) -> Result<(u64, String), String> {
        let pty_id = generate_pty_id();
        let session_key = uuid::Uuid::new_v4().to_string();

        let integration_dir = if exec_command.is_some() {
            None // No shell integration for non-interactive exec
        } else {
            app.path()
                .app_data_dir()
                .ok()
                .and_then(|d| shell_integration::create_shell_integration_files(&d).ok())
        };

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        let mut extra_env = Vec::new();
        if let Some(mcp_handle) = app.try_state::<crate::mcp::McpServerHandle>() {
            if let Some(info) = mcp_handle.connection_info() {
                extra_env.push(("RELEASH_MCP_URL".to_string(), info.url));
                extra_env.push(("RELEASH_MCP_TOKEN".to_string(), info.token));
            } else if let Some(app_config) =
                app.try_state::<std::sync::Arc<crate::config::AppConfig>>()
            {
                if let Ok(config) = app_config.get_config() {
                    let port = config.server.mcp_port;
                    let token = config.server.mcp_token.clone();
                    extra_env.push((
                        "RELEASH_MCP_URL".to_string(),
                        format!("http://127.0.0.1:{port}/mcp"),
                    ));
                    extra_env.push(("RELEASH_MCP_TOKEN".to_string(), token));
                }
            }
        }

        let config = SpawnConfig {
            rows,
            cols,
            cwd,
            shell,
            integration_dir,
            pty_id,
            extra_env,
            exec_command,
        };

        let backend_session = self.backend.spawn(config)?;
        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_BUFFER_CAPACITY)));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code_holder = Arc::new(Mutex::new(None::<i32>));

        let writer = backend_session.writer;
        let resizer = backend_session.resizer;
        let reader = backend_session.reader;
        let child = backend_session.child;
        let killer = child.clone_killer();

        let session = PtySession {
            writer,
            killer: Arc::new(Mutex::new(killer)),
            resizer,
            session_key: session_key.clone(),
            worktree_path,
            label,
            kind,
            output_buffer: Arc::clone(&output_buffer),
            exited: Arc::clone(&exited),
            exit_code: Arc::clone(&exit_code_holder),
        };

        self.sessions.lock().insert(pty_id, session);

        spawn_output_reader(
            app.clone(),
            pty_id,
            reader,
            child,
            output_buffer,
            exited,
            exit_code_holder,
        );

        Ok((pty_id, session_key))
    }
}

fn parse_pty_kind(kind: Option<&str>) -> PtyKind {
    match kind {
        Some("agent") => PtyKind::Agent,
        Some("one_shot") => PtyKind::OneShot,
        _ => PtyKind::Terminal,
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
#[allow(clippy::too_many_arguments)]
pub fn spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: Option<String>,
    label: Option<String>,
    kind: Option<String>,
) -> Result<u64, String> {
    let pty_kind = parse_pty_kind(kind.as_deref());
    let (pty_id, _session_key) =
        state.spawn(&app, rows, cols, cwd, worktree_path, label, pty_kind)?;
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
pub fn list_pty_sessions(state: State<'_, Arc<PtyManager>>) -> Vec<PtySessionInfo> {
    state.list_pty_sessions()
}

#[tauri::command]
pub fn kill_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    pty_id: u64,
) -> Result<(), String> {
    state.kill(pty_id)?;
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        ws.remove_pty_output_buffer(pty_id);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct GetOrSpawnPtyResult {
    pty_id: u64,
    session_key: String,
    buffered_output: String,
    is_new: bool,
    is_exited: bool,
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    kind: PtyKind,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    session_key: Option<String>,
    worktree_path: String,
    label: Option<String>,
    kind: Option<String>,
) -> Result<GetOrSpawnPtyResult, String> {
    let pty_kind = parse_pty_kind(kind.as_deref());

    if let Some(key) = &session_key {
        if let Some(found) = state.find_session(key) {
            return Ok(GetOrSpawnPtyResult {
                pty_id: found.pty_id,
                session_key: found.session_key,
                buffered_output: found.buffered_output,
                is_new: false,
                is_exited: found.is_exited,
                exit_code: found.exit_code,
                label: found.label,
                kind: found.kind,
            });
        }
    }

    // No existing session — spawn a new one
    let (pty_id, new_session_key) = state.spawn(
        &app,
        rows,
        cols,
        cwd,
        Some(worktree_path),
        label.clone(),
        pty_kind,
    )?;
    Ok(GetOrSpawnPtyResult {
        pty_id,
        session_key: new_session_key,
        buffered_output: String::new(),
        is_new: true,
        is_exited: false,
        exit_code: None,
        label,
        kind: pty_kind,
    })
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    worktree_path: String,
) -> Result<(), String> {
    let killed_ids = state.kill_by_worktree(&worktree_path);
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        for pty_id in killed_ids {
            ws.remove_pty_output_buffer(pty_id);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn gc_ptys_for_worktree(
    app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    worktree_path: String,
    keep_session_keys: Vec<String>,
    kind: Option<String>,
) -> Result<(), String> {
    let kind_filter = kind.as_deref().map(|k| parse_pty_kind(Some(k)));
    let killed_ids = state.gc_by_worktree(&worktree_path, &keep_session_keys, kind_filter);
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        for pty_id in killed_ids {
            ws.remove_pty_output_buffer(pty_id);
        }
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
            session_key: uuid::Uuid::new_v4().to_string(),
            worktree_path: Some("/repo".to_string()),
            label: Some("dev".to_string()),
            kind: PtyKind::Terminal,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"label\":\"dev\""));
        assert!(json.contains("\"session_key\""));
        let deserialized: PtySessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.label.unwrap(), "dev");
    }

    #[test]
    fn test_pty_session_info_serialization_without_label() {
        let info = PtySessionInfo {
            pty_id: 1,
            session_key: uuid::Uuid::new_v4().to_string(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            kind: PtyKind::Terminal,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("\"label\""));
        assert!(json.contains("\"session_key\""));
    }

    #[test]
    fn test_pty_session_info_serialization_with_session_key() {
        let key = uuid::Uuid::new_v4().to_string();
        let info = PtySessionInfo {
            pty_id: 1,
            session_key: key.clone(),
            worktree_path: Some("/repo".to_string()),
            label: None,
            kind: PtyKind::Agent,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains(&format!("\"session_key\":\"{}\"", key)));
        let deserialized: PtySessionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.session_key, key);
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
        assert_eq!(pm.backend_name(), "direct");
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
            session_key: uuid::Uuid::new_v4().to_string(),
            buffered_output: String::new(),
            is_new: true,
            is_exited: false,
            exit_code: None,
            label: None,
            kind: PtyKind::Terminal,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("\"label\""));
        assert!(json.contains("\"session_key\""));
        assert!(json.contains("\"kind\":\"terminal\""));
    }

    // ---- process_pty_output tests ----

    #[test]
    fn test_process_pty_output_valid_utf8() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let result = process_pty_output(b"hello world", &mut pending, &output_buffer, 1);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "hello world");
        assert!(pending.is_empty());
        let ring = output_buffer.lock();
        assert_eq!(ring.len(), 11);
    }

    #[test]
    fn test_process_pty_output_incomplete_utf8_pending() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        // First two bytes of a 3-byte UTF-8 sequence (e.g. 'あ' = 0xE3 0x81 0x82)
        let result = process_pty_output(&[0xE3, 0x81], &mut pending, &output_buffer, 1);
        assert!(result.is_none());
        assert_eq!(pending.len(), 2);
        // Now send the remaining byte
        let result = process_pty_output(&[0x82], &mut pending, &output_buffer, 1);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "あ");
        assert!(pending.is_empty());
    }

    #[test]
    fn test_process_pty_output_max_pending_drop() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        // Fill with invalid UTF-8 beyond MAX_PENDING_BYTES
        let invalid_bytes = vec![0xFF; MAX_PENDING_BYTES + 1];
        let result = process_pty_output(&invalid_bytes, &mut pending, &output_buffer, 1);
        assert!(result.is_none());
        assert!(pending.is_empty()); // Should be cleared
    }

    #[test]
    fn test_process_pty_output_invalid_below_max_pending_retained() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        // Small amount of invalid UTF-8 below the threshold
        let invalid_bytes = vec![0xFF; 10];
        let result = process_pty_output(&invalid_bytes, &mut pending, &output_buffer, 1);
        assert!(result.is_none());
        assert_eq!(pending.len(), 10); // Should be retained
    }

    #[test]
    fn test_process_pty_output_ring_buffer_overflow() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        // Fill to near capacity
        let data = "x".repeat(OUTPUT_BUFFER_CAPACITY - 10);
        process_pty_output(data.as_bytes(), &mut pending, &output_buffer, 1);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY - 10);
        // Add more to overflow
        let data2 = "y".repeat(20);
        process_pty_output(data2.as_bytes(), &mut pending, &output_buffer, 1);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY);
    }

    #[test]
    fn test_process_pty_output_exceeds_capacity() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        // Data larger than capacity — should truncate from the beginning
        let data = "z".repeat(OUTPUT_BUFFER_CAPACITY + 100);
        process_pty_output(data.as_bytes(), &mut pending, &output_buffer, 1);
        assert_eq!(output_buffer.lock().len(), OUTPUT_BUFFER_CAPACITY);
    }

    #[test]
    fn test_process_pty_output_empty_input() {
        let mut pending = Vec::new();
        let output_buffer = Mutex::new(VecDeque::new());
        let result = process_pty_output(b"", &mut pending, &output_buffer, 1);
        // Empty string from strip_osc_cmd_done should return None
        assert!(result.is_none());
    }

    // ---- Mock-based session tests ----

    struct MockWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct MockKiller {
        killed: AtomicBool,
    }

    impl portable_pty::ChildKiller for MockKiller {
        fn kill(&mut self) -> Result<(), std::io::Error> {
            self.killed.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(MockKiller {
                killed: AtomicBool::new(self.killed.load(Ordering::SeqCst)),
            })
        }
    }

    struct MockResizer {
        rows: u16,
        cols: u16,
    }

    impl backend::PtyResizer for MockResizer {
        fn resize(&mut self, rows: u16, cols: u16) -> Result<(), String> {
            self.rows = rows;
            self.cols = cols;
            Ok(())
        }
        fn get_size(&self) -> Result<(u16, u16), String> {
            Ok((self.cols, self.rows))
        }
    }

    fn insert_test_session(
        pm: &PtyManager,
        pty_id: u64,
        worktree_path: Option<&str>,
        label: Option<&str>,
    ) {
        insert_test_session_with_key(
            pm,
            pty_id,
            &uuid::Uuid::new_v4().to_string(),
            worktree_path,
            label,
            PtyKind::Terminal,
        );
    }

    fn insert_test_session_with_key(
        pm: &PtyManager,
        pty_id: u64,
        session_key: &str,
        worktree_path: Option<&str>,
        label: Option<&str>,
        kind: PtyKind,
    ) {
        let written = Arc::new(Mutex::new(Vec::<u8>::new()));
        let session = PtySession {
            writer: Arc::new(Mutex::new(Box::new(MockWriter(written)))),
            killer: Arc::new(Mutex::new(Box::new(MockKiller {
                killed: AtomicBool::new(false),
            }))),
            resizer: Arc::new(Mutex::new(Box::new(MockResizer { rows: 24, cols: 80 }))),
            session_key: session_key.to_string(),
            worktree_path: worktree_path.map(|s| s.to_string()),
            label: label.map(|s| s.to_string()),
            kind,
            output_buffer: Arc::new(Mutex::new(VecDeque::new())),
            exited: Arc::new(AtomicBool::new(false)),
            exit_code: Arc::new(Mutex::new(None)),
        };
        pm.sessions.lock().insert(pty_id, session);
    }

    #[test]
    fn test_write_success() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let result = pm.write(1, "hello");
        assert!(result.is_ok());
    }

    #[test]
    fn test_kill_success() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let result = pm.kill(1);
        assert!(result.is_ok());
        assert!(pm.sessions.lock().get(&1).is_none());
    }

    #[test]
    fn test_kill_already_exited() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        pm.sessions
            .lock()
            .get(&1)
            .unwrap()
            .exited
            .store(true, Ordering::SeqCst);
        let result = pm.kill(1);
        assert!(result.is_ok());
        assert!(pm.sessions.lock().get(&1).is_none());
    }

    #[test]
    fn test_resize_success() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let result = pm.resize(1, 30, 100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_pty_size_success() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let result = pm.get_pty_size(1);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), (80, 24));
    }

    #[test]
    fn test_list_pty_sessions_nonempty() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 10, Some("/repo"), Some("dev"));
        insert_test_session(&pm, 20, Some("/repo2"), None);
        let sessions = pm.list_pty_sessions();
        assert_eq!(sessions.len(), 2);
    }

    // ---- find_session tests ----

    #[test]
    fn test_find_session_by_uuid() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key, Some("/repo"), Some("dev"), PtyKind::Terminal);
        let found = pm.find_session(&key);
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.pty_id, 1);
        assert_eq!(found.session_key, key);
        assert!(!found.is_exited);
        assert_eq!(found.label, Some("dev".to_string()));
    }

    #[test]
    fn test_find_session_no_match() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), Some("dev"));
        let found = pm.find_session("nonexistent-uuid");
        assert!(found.is_none());
    }

    #[test]
    fn test_find_session_buffered_output() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key, Some("/repo"), None, PtyKind::Terminal);
        // Insert some data into the output buffer
        {
            let sessions = pm.sessions.lock();
            let session = sessions.get(&1).unwrap();
            session.output_buffer.lock().extend(b"buffered data");
        }
        let found = pm.find_session(&key).unwrap();
        assert_eq!(found.buffered_output, "buffered data");
    }

    #[test]
    fn test_find_session_uuid_exact_match_only() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key1 = uuid::Uuid::new_v4().to_string();
        let key2 = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key1, Some("/repo"), None, PtyKind::Terminal);
        insert_test_session_with_key(&pm, 2, &key2, Some("/repo"), None, PtyKind::Terminal);

        let found = pm.find_session(&key1).unwrap();
        assert_eq!(found.pty_id, 1);

        let found = pm.find_session(&key2).unwrap();
        assert_eq!(found.pty_id, 2);
    }

    // ---- kill_by_worktree tests ----

    #[test]
    fn test_kill_by_worktree_removes_matching() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), Some("dev"));
        insert_test_session(&pm, 2, Some("/repo"), Some("test"));
        insert_test_session(&pm, 3, Some("/other"), None);
        let killed = pm.kill_by_worktree("/repo");
        assert_eq!(killed.len(), 2);
        assert!(killed.contains(&1));
        assert!(killed.contains(&2));
        let sessions = pm.sessions.lock();
        assert_eq!(sessions.len(), 1);
        assert!(sessions.contains_key(&3));
    }

    #[test]
    fn test_kill_by_worktree_no_match() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let killed = pm.kill_by_worktree("/nonexistent");
        assert!(killed.is_empty());
        assert_eq!(pm.sessions.lock().len(), 1);
    }

    #[test]
    fn test_kill_by_worktree_empty_sessions() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let killed = pm.kill_by_worktree("/repo");
        assert!(killed.is_empty());
    }

    #[test]
    fn test_kill_by_worktree_already_exited() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        pm.sessions
            .lock()
            .get(&1)
            .unwrap()
            .exited
            .store(true, Ordering::SeqCst);
        let killed = pm.kill_by_worktree("/repo");
        assert_eq!(killed, vec![1]);
        assert!(pm.sessions.lock().is_empty());
    }

    // ---- gc_by_worktree tests ----

    #[test]
    fn test_gc_by_worktree_keeps_listed_keys() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key1 = uuid::Uuid::new_v4().to_string();
        let key2 = uuid::Uuid::new_v4().to_string();
        let key3 = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key1, Some("/repo"), Some("dev"), PtyKind::Terminal);
        insert_test_session_with_key(
            &pm,
            2,
            &key2,
            Some("/repo"),
            Some("test"),
            PtyKind::Terminal,
        );
        insert_test_session_with_key(&pm, 3, &key3, Some("/other"), None, PtyKind::Terminal);

        let killed = pm.gc_by_worktree("/repo", &[key1.clone()], None);
        // key2 のみ kill される（key1 は keep、key3 は別 worktree）
        assert_eq!(killed.len(), 1);
        assert!(killed.contains(&2));
        let sessions = pm.sessions.lock();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains_key(&1));
        assert!(sessions.contains_key(&3));
    }

    #[test]
    fn test_gc_by_worktree_empty_keep_list() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key1 = uuid::Uuid::new_v4().to_string();
        let key2 = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key1, Some("/repo"), None, PtyKind::Terminal);
        insert_test_session_with_key(&pm, 2, &key2, Some("/repo"), None, PtyKind::Terminal);

        let killed = pm.gc_by_worktree("/repo", &[], None);
        assert_eq!(killed.len(), 2);
        assert!(pm.sessions.lock().is_empty());
    }

    #[test]
    fn test_gc_by_worktree_no_match() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let killed = pm.gc_by_worktree("/nonexistent", &[], None);
        assert!(killed.is_empty());
        assert_eq!(pm.sessions.lock().len(), 1);
    }

    // ---- PtyKind tests ----

    #[test]
    fn test_pty_kind_serialization() {
        let agent_json = serde_json::to_string(&PtyKind::Agent).unwrap();
        assert_eq!(agent_json, "\"agent\"");
        let terminal_json = serde_json::to_string(&PtyKind::Terminal).unwrap();
        assert_eq!(terminal_json, "\"terminal\"");
        let oneshot_json = serde_json::to_string(&PtyKind::OneShot).unwrap();
        assert_eq!(oneshot_json, "\"one_shot\"");

        let deserialized: PtyKind = serde_json::from_str("\"agent\"").unwrap();
        assert_eq!(deserialized, PtyKind::Agent);
        let deserialized: PtyKind = serde_json::from_str("\"terminal\"").unwrap();
        assert_eq!(deserialized, PtyKind::Terminal);
        let deserialized: PtyKind = serde_json::from_str("\"one_shot\"").unwrap();
        assert_eq!(deserialized, PtyKind::OneShot);
    }

    #[test]
    fn test_remove_if_exited() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        pm.sessions
            .lock()
            .get(&1)
            .unwrap()
            .exited
            .store(true, Ordering::SeqCst);
        pm.remove_if_exited(1);
        assert!(pm.sessions.lock().get(&1).is_none());
    }

    #[test]
    fn test_remove_if_exited_running() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        pm.remove_if_exited(1);
        assert!(pm.sessions.lock().get(&1).is_some());
    }

    #[test]
    fn test_get_exit_status_running() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        let status = pm.get_exit_status(1);
        assert_eq!(status, Some((false, None)));
    }

    #[test]
    fn test_get_exit_status_exited() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        insert_test_session(&pm, 1, Some("/repo"), None);
        {
            let sessions = pm.sessions.lock();
            let s = sessions.get(&1).unwrap();
            s.exited.store(true, Ordering::SeqCst);
            *s.exit_code.lock() = Some(42);
        }
        let status = pm.get_exit_status(1);
        assert_eq!(status, Some((true, Some(42))));
    }

    #[test]
    fn test_get_exit_status_nonexistent() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let status = pm.get_exit_status(99999);
        assert!(status.is_none());
    }

    #[test]
    fn test_list_pty_sessions_includes_kind() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key, Some("/repo"), Some("agent"), PtyKind::Agent);
        insert_test_session(&pm, 2, Some("/repo"), Some("term"));
        let sessions = pm.list_pty_sessions();
        assert_eq!(sessions.len(), 2);
        let agent_session = sessions.iter().find(|s| s.pty_id == 1).unwrap();
        assert_eq!(agent_session.kind, PtyKind::Agent);
        let term_session = sessions.iter().find(|s| s.pty_id == 2).unwrap();
        assert_eq!(term_session.kind, PtyKind::Terminal);
    }

    #[test]
    fn test_gc_by_worktree_with_kind_filter() {
        let pm = PtyManager::with_backend(Box::new(DirectPtyBackend::new()));
        let key1 = uuid::Uuid::new_v4().to_string();
        let key2 = uuid::Uuid::new_v4().to_string();
        let key3 = uuid::Uuid::new_v4().to_string();
        insert_test_session_with_key(&pm, 1, &key1, Some("/repo"), None, PtyKind::Agent);
        insert_test_session_with_key(&pm, 2, &key2, Some("/repo"), None, PtyKind::Terminal);
        insert_test_session_with_key(&pm, 3, &key3, Some("/repo"), None, PtyKind::OneShot);

        // Only GC Agent kind
        let killed = pm.gc_by_worktree("/repo", &[], Some(PtyKind::Agent));
        assert_eq!(killed, vec![1]);
        assert_eq!(pm.sessions.lock().len(), 2);
    }

    #[test]
    fn test_parse_pty_kind() {
        assert_eq!(parse_pty_kind(Some("agent")), PtyKind::Agent);
        assert_eq!(parse_pty_kind(Some("one_shot")), PtyKind::OneShot);
        assert_eq!(parse_pty_kind(Some("terminal")), PtyKind::Terminal);
        assert_eq!(parse_pty_kind(Some("unknown")), PtyKind::Terminal);
        assert_eq!(parse_pty_kind(None), PtyKind::Terminal);
    }
}
