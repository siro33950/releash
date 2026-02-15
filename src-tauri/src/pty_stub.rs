#![allow(dead_code)]

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State};

const OUTPUT_BUFFER_CAPACITY: usize = 64 * 1024;

static PTY_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_pty_id() -> u64 {
    PTY_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<u64, ()>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PtySessionInfo {
    pub pty_id: u64,
    pub worktree_path: Option<String>,
}

impl PtyManager {
    pub fn write(&self, pty_id: u64, _data: &str) -> Result<(), String> {
        Err(format!("PTY {} not found", pty_id))
    }

    pub fn get_pty_size(&self, pty_id: u64) -> Result<(u16, u16), String> {
        Err(format!("PTY {} not found", pty_id))
    }

    pub fn list_pty_sessions(&self) -> Vec<PtySessionInfo> {
        Vec::new()
    }

    pub fn resize(&self, _pty_id: u64, rows: u16, cols: u16) -> Result<(), String> {
        if rows == 0 || cols == 0 {
            return Ok(());
        }
        Err("PTY feature is not enabled".to_string())
    }

    pub fn spawn(
        &self,
        _app: &AppHandle,
        _rows: u16,
        _cols: u16,
        _cwd: Option<String>,
        _worktree_path: Option<String>,
    ) -> Result<u64, String> {
        Err("PTY feature is not enabled".to_string())
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
    _app: AppHandle,
    _state: State<'_, Arc<PtyManager>>,
    _rows: u16,
    _cols: u16,
    _cwd: Option<String>,
    _worktree_path: Option<String>,
) -> Result<u64, String> {
    Err("PTY feature is not enabled".to_string())
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
    _app: AppHandle,
    state: State<'_, Arc<PtyManager>>,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    state.resize(pty_id, rows, cols)
}

#[tauri::command]
pub fn list_pty_sessions(state: State<'_, Arc<PtyManager>>) -> Vec<PtySessionInfo> {
    state.list_pty_sessions()
}

#[tauri::command]
pub fn kill_pty(_state: State<'_, Arc<PtyManager>>, pty_id: u64) -> Result<(), String> {
    Err(format!("PTY {} not found", pty_id))
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
    _app: AppHandle,
    _state: State<'_, Arc<PtyManager>>,
    _rows: u16,
    _cols: u16,
    _cwd: Option<String>,
    _worktree_path: String,
) -> Result<GetOrSpawnPtyResult, String> {
    Err("PTY feature is not enabled".to_string())
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    _state: State<'_, Arc<PtyManager>>,
    _worktree_path: String,
) -> Result<(), String> {
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
