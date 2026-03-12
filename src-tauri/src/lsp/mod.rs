pub mod bridge;
pub mod detect;
pub mod download;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State};
use tokio::process::{Child, ChildStdin};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

fn fnv1a_hash(input: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// Search Gradle and Maven caches for lombok.jar (excluding sources jars).
/// Returns the path to the newest version found, or None.
fn find_lombok_jar() -> Option<PathBuf> {
    let home = dirs::home_dir()?;

    let search_dirs = [
        home.join(".gradle/caches/modules-2/files-2.1/org.projectlombok/lombok"),
        home.join(".m2/repository/org/projectlombok/lombok"),
    ];

    let mut candidates: Vec<PathBuf> = Vec::new();

    for dir in &search_dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(walker) = walkdir(dir) {
            candidates.extend(walker);
        }
    }

    // Sort descending by version number so the newest version comes first
    candidates.sort_by(|a, b| {
        let a_ver = parse_lombok_version(&a.file_name().unwrap_or_default().to_string_lossy());
        let b_ver = parse_lombok_version(&b.file_name().unwrap_or_default().to_string_lossy());
        b_ver.cmp(&a_ver)
    });

    candidates.into_iter().next()
}

/// Parse version segments from a lombok jar filename (e.g. "lombok-1.18.38.jar" → [1, 18, 38]).
fn parse_lombok_version(filename: &str) -> Vec<u32> {
    let name = filename.strip_prefix("lombok-").unwrap_or(filename);
    let name = name.strip_suffix(".jar").unwrap_or(name);
    name.split('.')
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

/// Walk a directory tree to find lombok-*.jar files (excluding sources/javadoc jars).
fn walkdir(dir: &std::path::Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut results = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Ok(sub) = walkdir(&path) {
                results.extend(sub);
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("lombok-")
                && name.ends_with(".jar")
                && !name.contains("-sources")
                && !name.contains("-javadoc")
            {
                results.push(path);
            }
        }
    }
    Ok(results)
}

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
    stdin: Arc<Mutex<ChildStdin>>,
    status: LspStatus,
    command: String,
    pending_requests: bridge::PendingRequests,
    diagnostics_cache: bridge::DiagnosticsCache,
    request_id_counter: Arc<AtomicI64>,
    cancel_token: CancellationToken,
}

impl LspSession {
    async fn cleanup(&mut self) {
        self.cancel_token.cancel();
        self.diagnostics_cache.lock().await.clear();
        self.pending_requests.lock().await.clear();
    }
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
        on_message: Option<Channel<LspMessage>>,
    ) -> Result<u64, String> {
        // Check for existing session
        if let Some(session) = self.find_by_language(&worktree_path, &language).await {
            return Ok(session.id);
        }

        let mut final_args = args;
        let is_jdtls = language == "java" && command.contains("jdtls");
        if is_jdtls {
            if !final_args.iter().any(|a| a == "-data") {
                let data_dir = Self::jdtls_data_dir(app, &worktree_path)?;
                final_args.push("-data".to_string());
                final_args.push(data_dir.to_string_lossy().to_string());
            }
            // Auto-detect Lombok and inject as javaagent
            let has_lombok_arg = final_args
                .iter()
                .any(|a| a.contains("-javaagent") && a.contains("lombok"));
            if !has_lombok_arg {
                let lombok_path = tokio::task::spawn_blocking(find_lombok_jar)
                    .await
                    .map_err(|e| format!("Lombok 検出失敗: {e}"))?;
                if let Some(lombok_path) = lombok_path {
                    log::info!("Lombok detected: {}", lombok_path.display());
                    final_args.push(format!("--jvm-arg=-javaagent:{}", lombok_path.display()));
                }
            }
        }

        let mut cmd = tokio::process::Command::new(&command);
        cmd.args(&final_args)
            .current_dir(&worktree_path)
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
        let cancel_token = CancellationToken::new();

        let pending_requests: bridge::PendingRequests = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics_cache: bridge::DiagnosticsCache = Arc::new(Mutex::new(HashMap::new()));
        let request_id_counter = Arc::new(AtomicI64::new(1));

        // Spawn stdout reader task
        spawn_stdout_reader(
            id,
            stdout,
            on_message,
            pending_requests.clone(),
            diagnostics_cache.clone(),
            cancel_token.clone(),
        );

        // Spawn stderr logger
        if let Some(stderr) = stderr {
            let session_id = id;
            let app_handle = app.clone();
            let ct = cancel_token.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = ct.cancelled() => {},
                    _ = Self::stderr_logger(session_id, stderr, app_handle) => {},
                }
            });
        }

        // Monitor process exit
        let app_handle = app.clone();
        let manager = app.state::<Arc<LspManager>>().inner().clone();
        let session_id = id;
        let ct = cancel_token.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = ct.cancelled() => {},
                _ = Self::monitor_exit(session_id, manager, app_handle) => {},
            }
        });

        // Double-check: another task may have inserted a session while we were spawning
        {
            let sessions = self.sessions.lock().await;
            if let Some(existing) = sessions.values().find(|s| {
                s.worktree_path == worktree_path
                    && s.language == language
                    && s.status == LspStatus::Running
            }) {
                let existing_id = existing.id;
                drop(sessions);
                // Kill the newly spawned process since we already have one
                cancel_token.cancel();
                let mut child = child;
                let _ = child.kill().await;
                return Ok(existing_id);
            }
        }

        let session = LspSession {
            id,
            language,
            worktree_path,
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            status: LspStatus::Running,
            command,
            pending_requests,
            diagnostics_cache,
            request_id_counter,
            cancel_token,
        };

        self.sessions.lock().await.insert(id, session);

        Ok(id)
    }

    pub async fn shutdown(&self, session_id: u64) -> Result<(), String> {
        let mut session = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;

            if session.status != LspStatus::Running {
                return Ok(());
            }

            sessions.remove(&session_id).unwrap()
        };

        session.status = LspStatus::ShuttingDown;
        session.cleanup().await;

        // Send LSP shutdown request
        let shutdown_request = r#"{"jsonrpc":"2.0","id":99999,"method":"shutdown","params":null}"#;
        {
            let mut stdin = session.stdin.lock().await;
            if let Err(e) = bridge::write_to_stdin(&mut stdin, shutdown_request).await {
                log::warn!("LSP shutdown request failed for session {session_id}: {e}");
                let _ = session.child.kill().await;
                return Ok(());
            }

            // Send exit notification
            let exit_notification = r#"{"jsonrpc":"2.0","method":"exit","params":null}"#;
            let _ = bridge::write_to_stdin(&mut stdin, exit_notification).await;
        }

        // Wait briefly for the process to exit, then force kill (lock not held)
        let timeout =
            tokio::time::timeout(std::time::Duration::from_secs(5), session.child.wait()).await;

        if timeout.is_err() || timeout.is_ok_and(|r| r.is_err()) {
            let _ = session.child.kill().await;
        }

        Ok(())
    }

    pub async fn kill(&self, session_id: u64) -> Result<(), String> {
        let removed = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(&session_id)
        };
        if let Some(mut session) = removed {
            session.cleanup().await;
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
        let removed: Vec<LspSession> = {
            let mut sessions = self.sessions.lock().await;
            let ids_to_remove: Vec<u64> = sessions
                .values()
                .filter(|s| s.worktree_path == worktree_path)
                .map(|s| s.id)
                .collect();
            ids_to_remove
                .into_iter()
                .filter_map(|id| sessions.remove(&id))
                .collect()
        };
        for mut session in removed {
            session.cleanup().await;
            let _ = session.child.kill().await;
        }
    }

    pub async fn send_message(
        &self,
        session_id: u64,
        message: &str,
        worktree_path: &str,
    ) -> Result<(), String> {
        let stdin = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;

            if session.status != LspStatus::Running {
                return Err(format!("LSPセッション {session_id} は実行中ではありません"));
            }

            session.stdin.clone()
        };

        let message = bridge::inject_root_uri(message, worktree_path)?;
        let mut stdin_guard = stdin.lock().await;
        bridge::write_to_stdin(&mut stdin_guard, &message).await
    }

    // -----------------------------------------------------------------------
    // MCP-oriented methods
    // -----------------------------------------------------------------------

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request(
        &self,
        session_id: u64,
        method: &str,
        params: serde_json::Value,
        worktree_path: &str,
        timeout_ms: u64,
    ) -> Result<serde_json::Value, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Register the pending request (lock is scoped)
        let (request_id, pending_requests) = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;
            let id = session.request_id_counter.fetch_add(1, Ordering::Relaxed);
            session.pending_requests.lock().await.insert(id, tx);
            (id, session.pending_requests.clone())
        };

        let message = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });

        if let Err(e) = self
            .send_message(session_id, &message.to_string(), worktree_path)
            .await
        {
            pending_requests.lock().await.remove(&request_id);
            return Err(e);
        }

        let result =
            match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
                Ok(Ok(val)) => val,
                Ok(Err(_)) => {
                    pending_requests.lock().await.remove(&request_id);
                    return Err(format!(
                        "LSPリクエスト '{method}' のチャネルが閉じられました"
                    ));
                }
                Err(_) => {
                    pending_requests.lock().await.remove(&request_id);
                    return Err(format!(
                        "LSPリクエスト '{method}' がタイムアウト ({timeout_ms}ms)"
                    ));
                }
            };

        if let Some(error) = result.get("error") {
            return Err(format!("LSPエラー: {error}"));
        }

        Ok(result
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    /// Get cached diagnostics for a URI.
    #[allow(dead_code)]
    pub async fn get_cached_diagnostics(
        &self,
        session_id: u64,
        uri: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let diagnostics_cache = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;
            session.diagnostics_cache.clone()
        };
        let cache = diagnostics_cache.lock().await;
        Ok(cache.get(uri).cloned().unwrap_or_default())
    }

    /// Wait for diagnostics to appear in the cache for the given URI.
    /// Returns empty vec on timeout (does not error).
    pub async fn wait_for_diagnostics(
        &self,
        session_id: u64,
        uri: &str,
        timeout_ms: u64,
    ) -> Result<Vec<serde_json::Value>, String> {
        let diagnostics_cache = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&session_id)
                .ok_or(format!("LSPセッション {session_id} が見つかりません"))?;
            session.diagnostics_cache.clone()
        };

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);

        loop {
            {
                let cache = diagnostics_cache.lock().await;
                if let Some(diags) = cache.get(uri) {
                    return Ok(diags.clone());
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Ok(Vec::new());
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Ensure a running LSP session exists for the given language.
    /// Creates and initializes one if needed (headless, no Tauri channel).
    pub async fn ensure_session(
        &self,
        app: &AppHandle,
        worktree_path: &str,
        language: &str,
    ) -> Result<u64, String> {
        // Check for existing session
        if let Some(session) = self.find_by_language(worktree_path, language).await {
            return Ok(session.id);
        }

        // Detect LSP server
        let app_config = app.state::<Arc<crate::config::AppConfig>>();
        let cfg = app_config.get_config()?;
        let cache_dir = download::lsp_cache_dir(app)?;

        let server_config =
            detect::detect_server(language, &cfg.lsp, Some(&cache_dir), Some(worktree_path));

        let server_config = match server_config {
            Some(config) if !config.enabled => {
                return Err("LSPサーバーがユーザー設定で無効化されています".to_string());
            }
            Some(config) => config,
            None => download::install_lsp_server(app, language, &cache_dir)
                .await
                .map_err(|e| format!("LSPサーバーのインストール失敗: {e}"))?,
        };

        // Spawn headless (no channel)
        let id = self
            .spawn(
                app,
                worktree_path.to_string(),
                language.to_string(),
                server_config.command,
                server_config.args,
                None,
            )
            .await?;

        // Send initialize request
        let init_params = serde_json::json!({
            "processId": std::process::id(),
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {
                        "relatedInformation": true
                    },
                    "documentSymbol": {
                        "hierarchicalDocumentSymbolSupport": true
                    },
                    "hover": {
                        "contentFormat": ["markdown", "plaintext"]
                    },
                    "definition": {},
                    "references": {}
                }
            },
            "rootUri": null,
            "workspaceFolders": null,
        });

        if let Err(e) = self
            .request(id, "initialize", init_params, worktree_path, 30000)
            .await
        {
            let _ = self.kill(id).await;
            return Err(e);
        }

        // Send initialized notification
        let initialized_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        if let Err(e) = self
            .send_message(id, &initialized_msg.to_string(), worktree_path)
            .await
        {
            let _ = self.kill(id).await;
            return Err(e);
        }

        Ok(id)
    }

    // -----------------------------------------------------------------------

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

    fn jdtls_data_dir(app: &AppHandle, worktree_path: &str) -> Result<PathBuf, String> {
        let base = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app_data_dir 取得失敗: {e}"))?;
        let hash = fnv1a_hash(worktree_path);
        let dir = base.join("lsp").join("jdtls-workspaces").join(hash);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("JDT LSワークスペースディレクトリ作成失敗: {e}"))?;
        Ok(dir)
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
        .spawn(
            &app,
            worktree_path,
            language,
            command,
            args,
            Some(on_message),
        )
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

    #[tokio::test]
    async fn request_returns_error_for_nonexistent_session() {
        let manager = LspManager::default();
        let result = manager
            .request(999, "test", serde_json::json!({}), "/path", 1000)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_cached_diagnostics_returns_error_for_nonexistent_session() {
        let manager = LspManager::default();
        let result = manager.get_cached_diagnostics(999, "file:///test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wait_for_diagnostics_returns_error_for_nonexistent_session() {
        let manager = LspManager::default();
        let result = manager.wait_for_diagnostics(999, "file:///test", 100).await;
        assert!(result.is_err());
    }

    #[test]
    fn fnv1a_hash_is_deterministic() {
        let input = "/home/user/project";
        assert_eq!(fnv1a_hash(input), fnv1a_hash(input));
    }

    #[test]
    fn fnv1a_hash_differs_for_different_inputs() {
        let hash1 = fnv1a_hash("/home/user/project-a");
        let hash2 = fnv1a_hash("/home/user/project-b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn fnv1a_hash_length_is_16() {
        let hash = fnv1a_hash("/some/path");
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn walkdir_finds_lombok_jar() {
        let dir = tempfile::tempdir().unwrap();
        let version_dir = dir.path().join("1.18.38").join("abc123");
        std::fs::create_dir_all(&version_dir).unwrap();
        std::fs::write(version_dir.join("lombok-1.18.38.jar"), b"fake").unwrap();
        std::fs::write(version_dir.join("lombok-1.18.38-sources.jar"), b"fake").unwrap();

        let results = walkdir(dir.path()).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .contains("lombok-1.18.38.jar"));
    }

    #[test]
    fn walkdir_excludes_sources_and_javadoc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lombok-1.18.38-sources.jar"), b"fake").unwrap();
        std::fs::write(dir.path().join("lombok-1.18.38-javadoc.jar"), b"fake").unwrap();

        let results = walkdir(dir.path()).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn walkdir_returns_empty_for_nonexistent_dir() {
        let result = walkdir(std::path::Path::new("/nonexistent/dir/12345"));
        assert!(result.is_err());
    }
}
