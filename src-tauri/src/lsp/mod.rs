pub mod bridge;
pub mod detect;
pub mod download;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State};
use tokio::process::{Child, ChildStdin};
use tokio::sync::Mutex;

use bridge::spawn_stdout_reader;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

fn generate_session_id() -> u64 {
    NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspStatus {
    Running,
    ShuttingDown,
    Crashed,
}

struct LspSession {
    id: u64,
    language: String,
    worktree_path: String,
    child: Child,
    stdin: ChildStdin,
    status: LspStatus,
    command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSessionInfo {
    pub id: u64,
    pub language: String,
    pub worktree_path: String,
    pub status: LspStatus,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspMessage {
    pub session_id: u64,
    pub message: String,
}

pub struct LspManager {
    sessions: Mutex<HashMap<u64, LspSession>>,
}

impl Default for LspManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl LspManager {
    pub async fn spawn(
        &self,
        app: &AppHandle,
        worktree_path: String,
        language: String,
        command: String,
        args: Vec<String>,
        on_message: Channel<LspMessage>,
    ) -> Result<u64, String> {
        // Check for existing session
        if let Some(session) = self.find_by_language(&worktree_path, &language).await {
            return Ok(session.id);
        }

        let mut cmd = tokio::process::Command::new(&command);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("LSPサーバー起動失敗 ({command}): {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or("LSPサーバーのstdinを取得できません")?;

        let stdout = child
            .stdout
            .take()
            .ok_or("LSPサーバーのstdoutを取得できません")?;

        let stderr = child.stderr.take();

        let id = generate_session_id();

        // Spawn stdout reader task
        spawn_stdout_reader(id, stdout, on_message);

        // Spawn stderr logger
        if let Some(stderr) = stderr {
            let session_id = id;
            let app_handle = app.clone();
            tokio::spawn(async move {
                Self::stderr_logger(session_id, stderr, app_handle).await;
            });
        }

        // Monitor process exit
        let app_handle = app.clone();
        let manager = app.state::<Arc<LspManager>>().inner().clone();
        let session_id = id;
        tokio::spawn(async move {
            Self::monitor_exit(session_id, manager, app_handle).await;
        });

        let session = LspSession {
            id,
            language,
            worktree_path,
            child,
            stdin,
            status: LspStatus::Running,
            command,
        };

        self.sessions.lock().await.insert(id, session);

        Ok(id)
    }

    pub async fn shutdown(&self, session_id: u64) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;

        if session.status != LspStatus::Running {
            return Ok(());
        }

        session.status = LspStatus::ShuttingDown;

        // Send LSP shutdown request
        let shutdown_request = r#"{"jsonrpc":"2.0","id":99999,"method":"shutdown","params":null}"#;
        if let Err(e) = bridge::write_to_stdin(&mut session.stdin, shutdown_request).await {
            log::warn!("LSP shutdown request failed for session {session_id}: {e}");
            // Force kill if shutdown request fails
            let _ = session.child.kill().await;
            sessions.remove(&session_id);
            return Ok(());
        }

        // Send exit notification
        let exit_notification = r#"{"jsonrpc":"2.0","method":"exit","params":null}"#;
        let _ = bridge::write_to_stdin(&mut session.stdin, exit_notification).await;

        // Wait briefly for the process to exit, then force kill
        let child = &mut session.child;
        let timeout = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;

        match timeout {
            Ok(Ok(_)) => {
                sessions.remove(&session_id);
            }
            _ => {
                let _ = session.child.kill().await;
                sessions.remove(&session_id);
            }
        }

        Ok(())
    }

    pub async fn kill(&self, session_id: u64) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        if let Some(mut session) = sessions.remove(&session_id) {
            let _ = session.child.kill().await;
        }
        Ok(())
    }

    pub async fn list(&self) -> Vec<LspSessionInfo> {
        let sessions = self.sessions.lock().await;
        sessions
            .values()
            .map(|s| LspSessionInfo {
                id: s.id,
                language: s.language.clone(),
                worktree_path: s.worktree_path.clone(),
                status: s.status,
                command: s.command.clone(),
            })
            .collect()
    }

    pub async fn find_by_language(
        &self,
        worktree_path: &str,
        language: &str,
    ) -> Option<LspSessionInfo> {
        let sessions = self.sessions.lock().await;
        sessions.values().find_map(|s| {
            if s.worktree_path == worktree_path
                && s.language == language
                && s.status == LspStatus::Running
            {
                Some(LspSessionInfo {
                    id: s.id,
                    language: s.language.clone(),
                    worktree_path: s.worktree_path.clone(),
                    status: s.status,
                    command: s.command.clone(),
                })
            } else {
                None
            }
        })
    }

    pub async fn kill_by_worktree(&self, worktree_path: &str) {
        let mut sessions = self.sessions.lock().await;
        let ids_to_remove: Vec<u64> = sessions
            .values()
            .filter(|s| s.worktree_path == worktree_path)
            .map(|s| s.id)
            .collect();

        for id in ids_to_remove {
            if let Some(mut session) = sessions.remove(&id) {
                let _ = session.child.kill().await;
            }
        }
    }

    pub async fn send_message(
        &self,
        session_id: u64,
        message: &str,
        worktree_path: &str,
    ) -> Result<(), String> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;

        if session.status != LspStatus::Running {
            return Err(format!("LSPセッション {session_id} は実行中ではありません"));
        }

        let message = bridge::inject_root_uri(message, worktree_path)?;
        bridge::write_to_stdin(&mut session.stdin, &message).await
    }

    async fn stderr_logger(
        session_id: u64,
        stderr: tokio::process::ChildStderr,
        _app_handle: AppHandle,
    ) {
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
        while let Ok(Some(line)) = lines.next_line().await {
            log::debug!("LSP[{session_id}] stderr: {line}");
        }
    }

    async fn monitor_exit(session_id: u64, manager: Arc<LspManager>, app_handle: AppHandle) {
        // Wait until the process exits
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let mut sessions = manager.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&session_id) {
                match session.child.try_wait() {
                    Ok(Some(_status)) => {
                        if session.status == LspStatus::Running {
                            session.status = LspStatus::Crashed;
                            let _ = app_handle.emit(
                                "lsp-error",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "error": "LSPサーバーが予期せず終了しました",
                                }),
                            );
                        }
                        sessions.remove(&session_id);
                        break;
                    }
                    Ok(None) => {
                        // Still running
                    }
                    Err(e) => {
                        log::error!("LSP[{session_id}] wait error: {e}");
                        sessions.remove(&session_id);
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }
}

// --- Tauri Commands ---

#[tauri::command]
pub async fn spawn_lsp(
    app: AppHandle,
    state: State<'_, Arc<LspManager>>,
    worktree_path: String,
    language: String,
    command: String,
    args: Vec<String>,
    on_message: Channel<LspMessage>,
) -> Result<u64, String> {
    state
        .spawn(&app, worktree_path, language, command, args, on_message)
        .await
}

#[tauri::command]
pub async fn lsp_send(
    state: State<'_, Arc<LspManager>>,
    session_id: u64,
    message: String,
    worktree_path: String,
) -> Result<(), String> {
    state
        .send_message(session_id, &message, &worktree_path)
        .await
}

#[tauri::command]
pub async fn shutdown_lsp(
    state: State<'_, Arc<LspManager>>,
    session_id: u64,
) -> Result<(), String> {
    state.shutdown(session_id).await
}

#[tauri::command]
pub async fn kill_lsp(state: State<'_, Arc<LspManager>>, session_id: u64) -> Result<(), String> {
    state.kill(session_id).await
}

#[tauri::command]
pub async fn list_lsp_sessions(
    state: State<'_, Arc<LspManager>>,
) -> Result<Vec<LspSessionInfo>, String> {
    Ok(state.list().await)
}

#[tauri::command]
pub async fn kill_lsp_by_worktree(
    state: State<'_, Arc<LspManager>>,
    worktree_path: String,
) -> Result<(), String> {
    state.kill_by_worktree(&worktree_path).await;
    Ok(())
}

#[tauri::command]
pub fn detect_lsp_server(
    app: AppHandle,
    config_state: State<'_, Arc<crate::config::AppConfig>>,
    language: String,
    worktree_path: Option<String>,
) -> Result<Option<detect::LspServerConfig>, String> {
    let cfg = config_state.get_config()?;
    let cache_dir = download::lsp_cache_dir(&app)?;
    Ok(detect::detect_server(
        &language,
        &cfg.lsp,
        Some(&cache_dir),
        worktree_path.as_deref(),
    ))
}

#[tauri::command]
pub async fn install_lsp_server(
    app: AppHandle,
    language: String,
) -> Result<detect::LspServerConfig, String> {
    let cache_dir = download::lsp_cache_dir(&app)?;
    download::install_lsp_server(&app, &language, &cache_dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_language_for_extension(extension: String) -> Option<String> {
    detect::language_for_extension(&extension).map(String::from)
}

#[tauri::command]
pub fn get_supported_lsp_languages() -> Vec<String> {
    detect::supported_languages()
        .into_iter()
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_increments() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert!(id2 > id1);
    }

    #[test]
    fn lsp_status_serializes() {
        let json = serde_json::to_string(&LspStatus::Running).unwrap();
        assert_eq!(json, r#""running""#);

        let json = serde_json::to_string(&LspStatus::ShuttingDown).unwrap();
        assert_eq!(json, r#""shutting_down""#);

        let json = serde_json::to_string(&LspStatus::Crashed).unwrap();
        assert_eq!(json, r#""crashed""#);
    }

    #[tokio::test]
    async fn manager_default_has_no_sessions() {
        let manager = LspManager::default();
        assert!(manager.list().await.is_empty());
    }

    #[tokio::test]
    async fn find_by_language_returns_none_when_empty() {
        let manager = LspManager::default();
        assert!(manager
            .find_by_language("/path", "typescript")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn kill_nonexistent_session_is_ok() {
        let manager = LspManager::default();
        assert!(manager.kill(999).await.is_ok());
    }

    #[tokio::test]
    async fn shutdown_nonexistent_session_returns_error() {
        let manager = LspManager::default();
        assert!(manager.shutdown(999).await.is_err());
    }

    #[tokio::test]
    async fn kill_by_worktree_noop_when_empty() {
        let manager = LspManager::default();
        manager.kill_by_worktree("/nonexistent").await;
        assert!(manager.list().await.is_empty());
    }
}
