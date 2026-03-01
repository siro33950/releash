use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use super::PtyManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OneShotStatus {
    Starting,
    Running,
    Completed,
    Error,
    Cancelled,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneShotPtyInfo {
    pub pty_id: u64,
    pub session_key: String,
    pub worktree_path: String,
    pub label: String,
    pub status: OneShotStatus,
    pub exit_code: Option<i32>,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

pub struct OneShotPtyManager {
    entries: Mutex<HashMap<u64, OneShotPtyInfo>>,
    pty_manager: Arc<PtyManager>,
}

impl OneShotPtyManager {
    pub fn new(pty_manager: Arc<PtyManager>) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            pty_manager,
        }
    }

    pub fn spawn_oneshot(
        &self,
        app: &AppHandle,
        command: &str,
        worktree_path: &str,
        label: &str,
        timeout_secs: Option<u64>,
    ) -> Result<OneShotPtyInfo, String> {
        let (pty_id, session_key) = self.pty_manager.spawn_exec(
            app,
            50,
            200,
            Some(worktree_path.to_string()),
            Some(worktree_path.to_string()),
            Some(label.to_string()),
            command.to_string(),
        )?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let info = OneShotPtyInfo {
            pty_id,
            session_key: session_key.clone(),
            worktree_path: worktree_path.to_string(),
            label: label.to_string(),
            status: OneShotStatus::Running,
            exit_code: None,
            started_at: now,
            completed_at: None,
        };

        self.entries.lock().insert(pty_id, info.clone());
        let _ = app.emit("oneshot-pty-status-changed", &info);

        // Listen for pty-exit to update status
        let app_listen = app.clone();
        tauri::async_runtime::spawn(async move {
            Self::wait_for_exit(app_listen, pty_id, timeout_secs).await;
        });

        Ok(info)
    }

    async fn wait_for_exit(app: AppHandle, pty_id: u64, timeout_secs: Option<u64>) {
        let timeout = timeout_secs.unwrap_or(300);
        let (tx, rx) = tokio::sync::oneshot::channel::<Option<i32>>();
        let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

        let tx_clone = tx.clone();
        let listener = app.listen("pty-exit", move |event: tauri::Event| {
            if let Ok(exit) = serde_json::from_str::<super::PtyExit>(event.payload()) {
                if exit.pty_id == pty_id {
                    let tx_c = tx_clone.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(sender) = tx_c.lock().await.take() {
                            let _ = sender.send(exit.exit_code);
                        }
                    });
                }
            }
        });

        let result = tokio::select! {
            exit_code = rx => {
                match exit_code {
                    Ok(code) => {
                        let status = match code {
                            Some(0) => OneShotStatus::Completed,
                            _ => OneShotStatus::Error,
                        };
                        (status, code)
                    }
                    Err(_) => (OneShotStatus::Error, None),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                // Timeout — kill the PTY
                if let Some(mgr) = app.try_state::<Arc<PtyManager>>() {
                    let _ = mgr.kill(pty_id);
                }
                (OneShotStatus::Timeout, None)
            }
        };

        app.unlisten(listener);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        // Update the OneShotPtyManager state
        if let Some(mgr) = app.try_state::<Arc<OneShotPtyManager>>() {
            let mut entries = mgr.entries.lock();
            if let Some(entry) = entries.get_mut(&pty_id) {
                if entry.status != OneShotStatus::Cancelled {
                    entry.status = result.0;
                    entry.exit_code = result.1;
                }
                entry.completed_at = Some(now);
                let _ = app.emit("oneshot-pty-status-changed", &*entry);
            }
        }

        // Clean up completed entry after a delay
        let app_cleanup = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if let Some(mgr) = app_cleanup.try_state::<Arc<OneShotPtyManager>>() {
                mgr.entries.lock().remove(&pty_id);
            }
        });
    }

    pub fn cancel(&self, app: &AppHandle, pty_id: u64) -> Result<(), String> {
        self.pty_manager.kill(pty_id)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get_mut(&pty_id) {
            entry.status = OneShotStatus::Cancelled;
            entry.completed_at = Some(now);
            let _ = app.emit("oneshot-pty-status-changed", &*entry);
        }
        Ok(())
    }

    pub fn get_status(&self, pty_id: u64) -> Option<OneShotPtyInfo> {
        self.entries.lock().get(&pty_id).cloned()
    }

    pub fn list_active(&self) -> Vec<OneShotPtyInfo> {
        self.entries
            .lock()
            .values()
            .filter(|e| matches!(e.status, OneShotStatus::Starting | OneShotStatus::Running))
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn spawn_oneshot_pty(
    app: AppHandle,
    state: State<'_, Arc<OneShotPtyManager>>,
    command: String,
    worktree_path: String,
    label: String,
    timeout_secs: Option<u64>,
) -> Result<OneShotPtyInfo, String> {
    state.spawn_oneshot(&app, &command, &worktree_path, &label, timeout_secs)
}

#[tauri::command]
pub fn cancel_oneshot_pty(
    app: AppHandle,
    state: State<'_, Arc<OneShotPtyManager>>,
    pty_id: u64,
) -> Result<(), String> {
    state.cancel(&app, pty_id)
}

#[tauri::command]
pub fn get_oneshot_pty_status(
    state: State<'_, Arc<OneShotPtyManager>>,
    pty_id: u64,
) -> Option<OneShotPtyInfo> {
    state.get_status(pty_id)
}

#[tauri::command]
pub fn list_oneshot_ptys(state: State<'_, Arc<OneShotPtyManager>>) -> Vec<OneShotPtyInfo> {
    state.list_active()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oneshot_status_serialization() {
        let status = OneShotStatus::Completed;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"completed\"");

        let deserialized: OneShotStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, OneShotStatus::Completed);
    }

    #[test]
    fn oneshot_info_serialization() {
        let info = OneShotPtyInfo {
            pty_id: 1,
            session_key: "test-key".to_string(),
            worktree_path: "/repo".to_string(),
            label: "review".to_string(),
            status: OneShotStatus::Running,
            exit_code: None,
            started_at: 1234567890.0,
            completed_at: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"status\":\"running\""));
        assert!(json.contains("\"label\":\"review\""));

        let deserialized: OneShotPtyInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 1);
        assert_eq!(deserialized.status, OneShotStatus::Running);
    }

    #[test]
    fn oneshot_manager_new() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);
        assert!(mgr.list_active().is_empty());
    }

    #[test]
    fn get_status_nonexistent() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);
        assert!(mgr.get_status(99999).is_none());
    }
}
