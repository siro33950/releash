pub mod auth;
pub mod mcp_json;
pub mod server;
pub mod state;

use std::sync::Arc;

use axum::middleware;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tauri::Manager;
use tokio_util::sync::CancellationToken;

use self::server::ReleashMcpServer;
use self::state::McpSharedState;

// ---------------------------------------------------------------------------
// McpServerHandle — Tauri managed state
// ---------------------------------------------------------------------------

pub struct McpServerHandle {
    running: Arc<parking_lot::Mutex<bool>>,
    port: Arc<parking_lot::Mutex<Option<u16>>>,
    auth_token: Arc<parking_lot::Mutex<Option<String>>>,
    cancellation_token: Arc<parking_lot::Mutex<Option<CancellationToken>>>,
    server_task: Arc<tokio::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
}

impl Default for McpServerHandle {
    fn default() -> Self {
        Self {
            running: Arc::new(parking_lot::Mutex::new(false)),
            port: Arc::new(parking_lot::Mutex::new(None)),
            auth_token: Arc::new(parking_lot::Mutex::new(None)),
            cancellation_token: Arc::new(parking_lot::Mutex::new(None)),
            server_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct McpConnectionInfo {
    pub url: String,
    pub token: String,
}

impl McpServerHandle {
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }

    pub fn connection_info(&self) -> Option<McpConnectionInfo> {
        let running = self.running.lock();
        if !*running {
            return None;
        }
        let port = (*self.port.lock())?;
        let token = self.auth_token.lock().clone()?;
        Some(McpConnectionInfo {
            url: format!("http://127.0.0.1:{port}/mcp"),
            token,
        })
    }
}

// ---------------------------------------------------------------------------
// Core start / stop
// ---------------------------------------------------------------------------

pub async fn start_mcp_server_core(
    state: McpSharedState,
    handle: &McpServerHandle,
) -> Result<McpConnectionInfo, String> {
    {
        let mut running = handle.running.lock();
        if *running {
            return Err("MCP server is already running".to_string());
        }
        *running = true;
    }

    match start_mcp_server_inner(state, handle).await {
        Ok(info) => Ok(info),
        Err(e) => {
            *handle.running.lock() = false;
            Err(e)
        }
    }
}

async fn start_mcp_server_inner(
    state: McpSharedState,
    handle: &McpServerHandle,
) -> Result<McpConnectionInfo, String> {
    // config.toml から固定ポート/トークンを取得
    let config = state.app_config.get_config()?;
    let mcp_port = config.server.mcp_port;
    let token = config.server.mcp_token.clone();

    let ct = CancellationToken::new();

    let state_for_factory = state;
    let mut server_config = StreamableHttpServerConfig::default();
    server_config.cancellation_token = ct.child_token();
    let service = StreamableHttpService::new(
        move || Ok(ReleashMcpServer::new(state_for_factory.clone())),
        LocalSessionManager::default().into(),
        server_config,
    );

    let token_for_middleware = token.clone();
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            token_for_middleware,
            auth::auth_middleware,
        ));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{mcp_port}"))
        .await
        .map_err(|e| format!("Failed to bind MCP server on port {mcp_port}: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local addr: {e}"))?
        .port();

    let ct_for_shutdown = ct.clone();

    let info = McpConnectionInfo {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token: token.clone(),
    };

    {
        *handle.port.lock() = Some(port);
        *handle.auth_token.lock() = Some(token);
        *handle.cancellation_token.lock() = Some(ct);
    }

    let running_flag = handle.running.clone();
    let port_slot = handle.port.clone();
    let auth_slot = handle.auth_token.clone();
    let ct_slot = handle.cancellation_token.clone();

    let task = tauri::async_runtime::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                ct_for_shutdown.cancelled().await;
            })
            .await;

        if let Err(e) = result {
            log::error!("MCP server exited with error: {e}");
        }

        *running_flag.lock() = false;
        *port_slot.lock() = None;
        *auth_slot.lock() = None;
        *ct_slot.lock() = None;
    });
    *handle.server_task.lock().await = Some(task);

    log::info!("MCP server started on 127.0.0.1:{port}");
    Ok(info)
}

pub async fn stop_mcp_server_core(handle: &McpServerHandle) -> Result<(), String> {
    if !handle.is_running() {
        return Err("MCP server is not running".to_string());
    }

    if let Some(ct) = handle.cancellation_token.lock().take() {
        ct.cancel();
    }

    if let Some(task) = handle.server_task.lock().await.take() {
        let _ = task.await;
    }

    *handle.running.lock() = false;
    *handle.port.lock() = None;
    *handle.auth_token.lock() = None;

    log::info!("MCP server stopped");
    Ok(())
}

// ---------------------------------------------------------------------------
// Restart helper — stop → start with latest config
// ---------------------------------------------------------------------------

pub async fn restart_mcp_server_if_running(
    app: &tauri::AppHandle,
) -> Result<McpConnectionInfo, String> {
    let handle = app.state::<McpServerHandle>();

    if handle.is_running() {
        if let Err(e) = stop_mcp_server_core(&handle).await {
            // Stop failed — check if it's actually still running
            if handle.is_running() {
                return Err(format!(
                    "設定は保存しましたが、MCPサーバーの停止に失敗しました: {e}"
                ));
            }
            // Already stopped — proceed
        }
    }

    let state = build_mcp_state(app)
        .map_err(|e| format!("設定は保存しましたが、MCPサーバーの起動に失敗しました: {e}"))?;

    start_mcp_server_core(state, &handle)
        .await
        .map_err(|e| format!("設定は保存しましたが、MCPサーバーの起動に失敗しました: {e}"))
}

// ---------------------------------------------------------------------------
// Shared state builder
// ---------------------------------------------------------------------------

fn build_mcp_state(app: &tauri::AppHandle) -> Result<McpSharedState, String> {
    let pty_manager = app.state::<Arc<crate::pty::PtyManager>>();
    let app_config = app.state::<Arc<crate::config::AppConfig>>();
    let broadcaster = app.state::<Arc<crate::ws_bridge::WsBroadcaster>>();
    let comment_store = app.state::<Arc<crate::comment_store::CommentStore>>();
    let thread_store = app.state::<Arc<crate::thread_store::ThreadStore>>();
    let shared_repo_paths = app.state::<crate::repo_registry::SharedRepoPaths>();

    let app_data_dir = app.path().app_data_dir().ok();

    Ok(McpSharedState {
        repo_paths: Arc::clone(shared_repo_paths.inner()),
        pty_manager: Arc::clone(&pty_manager),
        app_config: Arc::clone(app_config.inner()),
        broadcaster: Arc::clone(&broadcaster),
        comment_store: Arc::clone(comment_store.inner()),
        thread_store: Arc::clone(thread_store.inner()),
        app_handle: Some(app.clone()),
        app_data_dir,
    })
}

// ---------------------------------------------------------------------------
// Auto-start helper (called from setup with AppHandle)
// ---------------------------------------------------------------------------

pub async fn auto_start_mcp_server(app: &tauri::AppHandle) -> Result<McpConnectionInfo, String> {
    let handle = app.state::<McpServerHandle>();
    let state = build_mcp_state(app)?;
    start_mcp_server_core(state, &handle).await
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_mcp_server(app: tauri::AppHandle) -> Result<McpConnectionInfo, String> {
    let handle = app.state::<McpServerHandle>();
    let state = build_mcp_state(&app)?;
    start_mcp_server_core(state, &handle).await
}

#[tauri::command]
pub async fn stop_mcp_server(handle: tauri::State<'_, McpServerHandle>) -> Result<(), String> {
    stop_mcp_server_core(&handle).await
}

#[derive(serde::Serialize)]
pub struct McpServerStatus {
    pub running: bool,
    pub port: Option<u16>,
}

#[tauri::command]
pub fn get_mcp_server_status(handle: tauri::State<'_, McpServerHandle>) -> McpServerStatus {
    McpServerStatus {
        running: handle.is_running(),
        port: *handle.port.lock(),
    }
}

#[tauri::command]
pub fn get_mcp_connection_info(
    handle: tauri::State<'_, McpServerHandle>,
) -> Option<McpConnectionInfo> {
    handle.connection_info()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_default_is_not_running() {
        let handle = McpServerHandle::default();
        assert!(!handle.is_running());
        assert!(handle.connection_info().is_none());
    }

    #[test]
    fn handle_connection_info_when_running() {
        let handle = McpServerHandle::default();
        *handle.running.lock() = true;
        *handle.port.lock() = Some(12345);
        *handle.auth_token.lock() = Some("test-token".to_string());

        let info = handle.connection_info().unwrap();
        assert_eq!(info.url, "http://127.0.0.1:12345/mcp");
        assert_eq!(info.token, "test-token");
    }

    #[test]
    fn handle_connection_info_none_when_not_running() {
        let handle = McpServerHandle::default();
        *handle.port.lock() = Some(12345);
        *handle.auth_token.lock() = Some("test-token".to_string());

        assert!(handle.connection_info().is_none());
    }

    #[tokio::test]
    async fn stop_when_not_running_returns_error() {
        let handle = McpServerHandle::default();
        let result = stop_mcp_server_core(&handle).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stop_cancels_and_clears_state() {
        let handle = McpServerHandle::default();
        let ct = CancellationToken::new();
        let ct_child = ct.child_token();

        *handle.running.lock() = true;
        *handle.port.lock() = Some(9999);
        *handle.auth_token.lock() = Some("tok".to_string());
        *handle.cancellation_token.lock() = Some(ct);

        let result = stop_mcp_server_core(&handle).await;
        assert!(result.is_ok());
        assert!(!handle.is_running());
        assert!(handle.port.lock().is_none());
        assert!(handle.auth_token.lock().is_none());
        assert!(ct_child.is_cancelled());
    }
}
