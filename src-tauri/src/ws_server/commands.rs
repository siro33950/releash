use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths;
use crate::domain::app_config::ConfigRepository;
use crate::ws_bridge::WsBroadcaster;

use super::http::start_ws_server;
use super::{StartServerResult, WsServerHandle, WsServerState};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerStatusPayload {
    pub running: bool,
    pub bound_ip: Option<String>,
    pub connection_mode: Option<String>,
}

pub async fn start_server_core(
    app: &tauri::AppHandle,
    bind_ip: String,
) -> Result<StartServerResult, String> {
    let handle = app.state::<WsServerHandle>();
    let config_state = app.state::<Arc<dyn ConfigRepository>>();
    let broadcaster = app.state::<Arc<WsBroadcaster>>();
    let pty_session_runtime_gateway = app
        .state::<Arc<crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway>>(
        );
    let pr_cache = app.state::<Arc<crate::git_host::PrCache>>();
    let shared_repo_paths = app.state::<SharedRepoPaths>();

    {
        let running = handle.running.lock();
        if *running {
            return Err("サーバーは既に起動しています".to_string());
        }
    }

    let mut cfg = config_state.load().map_err(|e| e.to_string())?;

    let detected = crate::adaptor::gateway::remote_access::network_impl::detect_all_interfaces();
    let mode = if detected.iter().any(|i| i.kind == "vpn" && i.ip == bind_ip) {
        "vpn".to_string()
    } else {
        "lan".to_string()
    };

    cfg.server.bind = bind_ip.clone();

    let bind_ip_addr: std::net::IpAddr = bind_ip
        .parse()
        .map_err(|e| format!("IPアドレスのパース失敗: {e}"))?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("データディレクトリの取得失敗: {e}"))?;
    if cfg.server.tls.cert.is_empty() || cfg.server.tls.key.is_empty() {
        let cert_gateway =
            crate::adaptor::gateway::remote_access::certificate_impl::TlsCertificateGateway;
        let (cert_path, key_path) =
            crate::usecase::remote_access::certificate_usecase::ensure_self_signed_cert(
                &cert_gateway,
                bind_ip_addr,
                &data_dir,
            )
            .map_err(|e| e.to_string())?;
        cfg.server.tls.cert = cert_path.to_string_lossy().to_string();
        cfg.server.tls.key = key_path.to_string_lossy().to_string();
    }
    cfg.server.tls.enabled = true;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let remote_dir = if cfg!(debug_assertions) {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("generated")
            .join("remote");
        if dir.exists() {
            Some(dir)
        } else {
            None
        }
    } else {
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("generated").join("remote"))
    };
    let backend_registry =
        app.state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>();
    // composition root（lib.rs）で組み立てた単一 RepositoryUsecase を注入する。
    // ws_server は routing/transport state に閉じ、DI 配線は持たない。
    let repository_usecase =
        app.state::<Arc<crate::usecase::repository_usecase::RepositoryUsecase>>();
    let server_state = Arc::new(WsServerState::new(
        remote_dir,
        Arc::clone(&broadcaster),
        Some(Arc::clone(&pty_session_runtime_gateway)),
        Arc::clone(shared_repo_paths.inner()),
        Arc::clone(config_state.inner()),
        Some(app.clone()),
        cfg.server.tls.enabled,
        Arc::clone(&pr_cache),
        Arc::clone(backend_registry.inner()),
        Arc::clone(repository_usecase.inner()),
    ));

    start_ws_server(&cfg, Arc::clone(&server_state), shutdown_rx).await?;

    {
        let mut running = handle.running.lock();
        *running = true;
        let mut tx = handle.shutdown_tx.lock();
        *tx = Some(shutdown_tx);
        handle.active_bind.lock().replace(bind_ip.clone());
        *handle.tls_enabled.lock() = cfg.server.tls.enabled;
        handle.connection_mode.lock().replace(mode.clone());
        *handle.server_state.lock() = Some(server_state);
    }

    let _ = app.emit(
        "server-status-changed",
        ServerStatusPayload {
            running: true,
            bound_ip: Some(bind_ip.clone()),
            connection_mode: Some(mode.clone()),
        },
    );

    // Save last bind IP
    if let Ok(mut config) = config_state.load() {
        config.app.last_bind_ip = bind_ip.clone();
        let _ = config_state.save(config);
    }

    Ok(StartServerResult { ip: bind_ip, mode })
}

pub async fn stop_server_core(app: &tauri::AppHandle) -> Result<(), String> {
    let handle = app.state::<WsServerHandle>();

    let tx = {
        let mut shutdown_tx = handle.shutdown_tx.lock();
        shutdown_tx.take()
    };

    if let Some(tx) = tx {
        let _ = tx.send(());
        let mut running = handle.running.lock();
        *running = false;
        handle.active_bind.lock().take();
        *handle.tls_enabled.lock() = false;
        handle.connection_mode.lock().take();
        *handle.server_state.lock() = None;

        let _ = app.emit(
            "server-status-changed",
            ServerStatusPayload {
                running: false,
                bound_ip: None,
                connection_mode: None,
            },
        );

        Ok(())
    } else {
        Err("サーバーは起動していません".to_string())
    }
}

#[tauri::command]
pub async fn start_server(
    bind_ip: String,
    app: tauri::AppHandle,
) -> Result<StartServerResult, String> {
    start_server_core(&app, bind_ip).await
}

#[tauri::command]
pub async fn stop_server(app: tauri::AppHandle) -> Result<(), String> {
    stop_server_core(&app).await
}

#[tauri::command]
pub fn get_server_status(handle: tauri::State<'_, WsServerHandle>) -> bool {
    *handle.running.lock()
}

#[derive(serde::Serialize)]
pub struct ServerInfo {
    pub running: bool,
    pub bound_ip: Option<String>,
    pub connection_mode: Option<String>,
}

#[tauri::command]
pub fn get_server_info(handle: tauri::State<'_, WsServerHandle>) -> ServerInfo {
    ServerInfo {
        running: handle.is_running(),
        bound_ip: handle.active_bind(),
        connection_mode: handle.connection_mode(),
    }
}

#[tauri::command]
pub fn update_terminal_startup_command(
    command: String,
    handle: tauri::State<'_, WsServerHandle>,
) -> Result<(), String> {
    let server_state = {
        let guard = handle.server_state.lock();
        guard.clone().ok_or("サーバーが起動していません")?
    };
    server_state.set_terminal_startup_command(command);
    Ok(())
}
