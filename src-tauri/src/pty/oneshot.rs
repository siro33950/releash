use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use super::PtyManager;

const MAX_ONESHOT_OUTPUT: usize = 1_048_576; // 1MB

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

struct OneShotPtyEntry {
    info: OneShotPtyInfo,
    output: String,
    output_listener_id: Option<tauri::EventId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FindOneShotPtyResult {
    #[serde(flatten)]
    pub info: OneShotPtyInfo,
    pub buffered_output: String,
}

pub struct OneShotPtyManager {
    entries: Mutex<HashMap<u64, OneShotPtyEntry>>,
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
            super::PtyKind::OneShot,
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

        self.entries.lock().insert(
            pty_id,
            OneShotPtyEntry {
                info: info.clone(),
                output: String::new(),
                output_listener_id: None,
            },
        );
        let _ = app.emit("oneshot-pty-status-changed", &info);

        // Register pty-output listener to accumulate output in the entry
        let app_for_output = app.clone();
        let output_listener = app.listen("pty-output", move |event| {
            if let Ok(payload) = serde_json::from_str::<super::PtyOutput>(event.payload()) {
                if payload.pty_id == pty_id {
                    if let Some(mgr) = app_for_output.try_state::<Arc<OneShotPtyManager>>() {
                        mgr.append_output(pty_id, &payload.data);
                    }
                }
            }
        });

        if let Some(e) = self.entries.lock().get_mut(&pty_id) {
            e.output_listener_id = Some(output_listener);
        }

        // Listen for pty-exit to update status
        let app_listen = app.clone();
        let pty_manager_clone = Arc::clone(&self.pty_manager);
        tauri::async_runtime::spawn(async move {
            Self::wait_for_exit(app_listen, pty_id, timeout_secs, pty_manager_clone).await;
        });

        Ok(info)
    }

    fn append_output(&self, pty_id: u64, data: &str) {
        let mut entries = self.entries.lock();
        if let Some(entry) = entries.get_mut(&pty_id) {
            entry.output.push_str(data);
            if entry.output.len() > MAX_ONESHOT_OUTPUT {
                let drain = entry.output.len() - MAX_ONESHOT_OUTPUT;
                let mut boundary = drain;
                while boundary < entry.output.len() && !entry.output.is_char_boundary(boundary) {
                    boundary += 1;
                }
                entry.output.drain(..boundary);
            }
        }
    }

    async fn wait_for_exit(
        app: AppHandle,
        pty_id: u64,
        timeout_secs: Option<u64>,
        pty_manager: Arc<PtyManager>,
    ) {
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

        // Fallback: check if the PTY already exited before listener was registered
        if let Some((true, exit_code)) = pty_manager.get_exit_status(pty_id) {
            let tx_fb = tx.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(sender) = tx_fb.lock().await.take() {
                    let _ = sender.send(exit_code);
                }
            });
        }

        let map_exit = |exit_code: Result<Option<i32>, _>| match exit_code {
            Ok(code) => {
                let status = match code {
                    Some(0) => OneShotStatus::Completed,
                    _ => OneShotStatus::Error,
                };
                (status, code)
            }
            Err(_) => (OneShotStatus::Error, None),
        };

        let result = if let Some(timeout) = timeout_secs {
            tokio::select! {
                exit_code = rx => map_exit(exit_code),
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    if let Some(mgr) = app.try_state::<Arc<PtyManager>>() {
                        let _ = mgr.kill(pty_id);
                    }
                    (OneShotStatus::Timeout, None)
                }
            }
        } else {
            map_exit(rx.await)
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
                if entry.info.status != OneShotStatus::Cancelled {
                    entry.info.status = result.0;
                    entry.info.exit_code = result.1;
                }
                entry.info.completed_at = Some(now);
                if let Some(listener_id) = entry.output_listener_id.take() {
                    app.unlisten(listener_id);
                }
                let _ = app.emit("oneshot-pty-status-changed", &entry.info);
            }
        }

        // Clean up completed entry after a delay (10 minutes)
        let app_cleanup = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
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
            entry.info.status = OneShotStatus::Cancelled;
            entry.info.completed_at = Some(now);
            let _ = app.emit("oneshot-pty-status-changed", &entry.info);
        }
        Ok(())
    }

    pub fn get_status(&self, pty_id: u64) -> Option<OneShotPtyInfo> {
        self.entries.lock().get(&pty_id).map(|e| e.info.clone())
    }

    pub fn list_active_for_worktree(&self, worktree_path: &str) -> Vec<FindOneShotPtyResult> {
        self.entries
            .lock()
            .values()
            .filter(|e| {
                e.info.worktree_path == worktree_path
                    && matches!(
                        e.info.status,
                        OneShotStatus::Starting | OneShotStatus::Running
                    )
            })
            .map(|e| {
                let buffered_output = if e.output.is_empty() {
                    self.pty_manager
                        .find_session(&e.info.session_key)
                        .map(|s| s.buffered_output)
                        .unwrap_or_default()
                } else {
                    e.output.clone()
                };
                FindOneShotPtyResult {
                    info: e.info.clone(),
                    buffered_output,
                }
            })
            .collect()
    }

    pub fn find_by_worktree_and_label(
        &self,
        worktree_path: &str,
        label: &str,
    ) -> Option<FindOneShotPtyResult> {
        self.entries
            .lock()
            .values()
            .filter(|e| e.info.worktree_path == worktree_path && e.info.label == label)
            .max_by(|a, b| {
                a.info
                    .started_at
                    .partial_cmp(&b.info.started_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|e| {
                let buffered_output = if e.output.is_empty() {
                    self.pty_manager
                        .find_session(&e.info.session_key)
                        .map(|s| s.buffered_output)
                        .unwrap_or_default()
                } else {
                    e.output.clone()
                };
                FindOneShotPtyResult {
                    info: e.info.clone(),
                    buffered_output,
                }
            })
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
pub fn list_oneshot_ptys(
    state: State<'_, Arc<OneShotPtyManager>>,
    worktree_path: String,
) -> Vec<FindOneShotPtyResult> {
    state.list_active_for_worktree(&worktree_path)
}

#[tauri::command]
pub fn find_oneshot_pty(
    state: State<'_, Arc<OneShotPtyManager>>,
    worktree_path: String,
    label: String,
) -> Option<FindOneShotPtyResult> {
    state.find_by_worktree_and_label(&worktree_path, &label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(
        pty_id: u64,
        worktree_path: &str,
        label: &str,
        status: OneShotStatus,
    ) -> OneShotPtyEntry {
        OneShotPtyEntry {
            info: OneShotPtyInfo {
                pty_id,
                session_key: format!("key-{}", pty_id),
                worktree_path: worktree_path.to_string(),
                label: label.to_string(),
                status,
                exit_code: None,
                started_at: 0.0,
                completed_at: None,
            },
            output: String::new(),
            output_listener_id: None,
        }
    }

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
        assert!(mgr.list_active_for_worktree("/repo").is_empty());
    }

    #[test]
    fn get_status_nonexistent() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);
        assert!(mgr.get_status(99999).is_none());
    }

    #[test]
    fn test_active_pty_ids_empty() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);
        assert!(mgr.list_active_for_worktree("/repo").is_empty());
    }

    #[test]
    fn test_active_pty_ids_filters_by_status() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        {
            let mut entries = mgr.entries.lock();
            entries.insert(1, make_entry(1, "/repo", "running", OneShotStatus::Running));
            entries.insert(2, {
                let mut e = make_entry(2, "/repo", "completed", OneShotStatus::Completed);
                e.info.exit_code = Some(0);
                e.info.completed_at = Some(1.0);
                e
            });
            entries.insert(
                3,
                make_entry(3, "/repo", "starting", OneShotStatus::Starting),
            );
        }

        let active = mgr.list_active_for_worktree("/repo");
        assert_eq!(active.len(), 2);
        let ids: Vec<u64> = active.iter().map(|e| e.info.pty_id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn list_active_for_worktree_filters_by_path() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        {
            let mut entries = mgr.entries.lock();
            entries.insert(
                1,
                make_entry(1, "/repo-a", "review:src/a.ts", OneShotStatus::Running),
            );
            entries.insert(
                2,
                make_entry(2, "/repo-b", "review:src/b.ts", OneShotStatus::Running),
            );
            entries.insert(
                3,
                make_entry(3, "/repo-a", "review:src/c.ts", OneShotStatus::Starting),
            );
            entries.insert(4, {
                let mut e = make_entry(4, "/repo-a", "review:src/d.ts", OneShotStatus::Completed);
                e.info.exit_code = Some(0);
                e.info.completed_at = Some(1.0);
                e
            });
        }

        let active_a = mgr.list_active_for_worktree("/repo-a");
        assert_eq!(active_a.len(), 2);
        let ids_a: Vec<u64> = active_a.iter().map(|e| e.info.pty_id).collect();
        assert!(ids_a.contains(&1));
        assert!(ids_a.contains(&3));

        let active_b = mgr.list_active_for_worktree("/repo-b");
        assert_eq!(active_b.len(), 1);
        assert_eq!(active_b[0].info.pty_id, 2);

        let active_c = mgr.list_active_for_worktree("/repo-c");
        assert!(active_c.is_empty());
    }

    #[test]
    fn find_by_worktree_and_label_found() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        mgr.entries
            .lock()
            .insert(1, make_entry(1, "/repo", "review", OneShotStatus::Running));

        let result = mgr.find_by_worktree_and_label("/repo", "review");
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.info.pty_id, 1);
        assert_eq!(result.info.worktree_path, "/repo");
        assert_eq!(result.info.label, "review");
    }

    #[test]
    fn find_by_worktree_and_label_not_found() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        let result = mgr.find_by_worktree_and_label("/repo", "review");
        assert!(result.is_none());
    }

    #[test]
    fn find_by_worktree_and_label_returns_latest() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        {
            let mut entries = mgr.entries.lock();

            let mut old = make_entry(1, "/repo", "review", OneShotStatus::Completed);
            old.info.started_at = 1000.0;
            entries.insert(1, old);

            let mut newer = make_entry(2, "/repo", "review", OneShotStatus::Running);
            newer.info.started_at = 2000.0;
            entries.insert(2, newer);

            // Different label — should not interfere
            let mut other = make_entry(3, "/repo", "agent", OneShotStatus::Running);
            other.info.started_at = 9999.0;
            entries.insert(3, other);
        }

        let result = mgr.find_by_worktree_and_label("/repo", "review");
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.info.pty_id, 2);
        assert_eq!(result.info.started_at, 2000.0);
    }

    #[test]
    fn find_by_worktree_and_label_wrong_label() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        mgr.entries
            .lock()
            .insert(1, make_entry(1, "/repo", "agent", OneShotStatus::Running));

        let result = mgr.find_by_worktree_and_label("/repo", "review");
        assert!(result.is_none());
    }

    #[test]
    fn append_output_basic() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        mgr.entries
            .lock()
            .insert(1, make_entry(1, "/repo", "review", OneShotStatus::Running));

        mgr.append_output(1, "hello ");
        mgr.append_output(1, "world");

        let entries = mgr.entries.lock();
        assert_eq!(entries.get(&1).unwrap().output, "hello world");
    }

    #[test]
    fn append_output_cap() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        mgr.entries
            .lock()
            .insert(1, make_entry(1, "/repo", "review", OneShotStatus::Running));

        // Fill to exactly MAX
        let big = "x".repeat(MAX_ONESHOT_OUTPUT);
        mgr.append_output(1, &big);
        assert_eq!(
            mgr.entries.lock().get(&1).unwrap().output.len(),
            MAX_ONESHOT_OUTPUT
        );

        // Append more — should trim from the front
        mgr.append_output(1, "extra");
        let output_len = mgr.entries.lock().get(&1).unwrap().output.len();
        assert!(output_len <= MAX_ONESHOT_OUTPUT);
        assert!(mgr
            .entries
            .lock()
            .get(&1)
            .unwrap()
            .output
            .ends_with("extra"));
    }

    #[test]
    fn find_result_includes_output() {
        let pm = Arc::new(PtyManager::default());
        let mgr = OneShotPtyManager::new(pm);

        mgr.entries
            .lock()
            .insert(1, make_entry(1, "/repo", "review", OneShotStatus::Running));
        mgr.append_output(1, "some output");

        let result = mgr.find_by_worktree_and_label("/repo", "review").unwrap();
        assert_eq!(result.buffered_output, "some output");
    }

    #[test]
    fn find_result_serialization() {
        let result = FindOneShotPtyResult {
            info: OneShotPtyInfo {
                pty_id: 1,
                session_key: "key-1".to_string(),
                worktree_path: "/repo".to_string(),
                label: "review".to_string(),
                status: OneShotStatus::Running,
                exit_code: None,
                started_at: 1234567890.0,
                completed_at: None,
            },
            buffered_output: "output".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        // #[serde(flatten)] should put info fields at the top level
        assert!(json.contains("\"pty_id\":1"));
        assert!(json.contains("\"buffered_output\":\"output\""));
        assert!(json.contains("\"status\":\"running\""));
    }
}
