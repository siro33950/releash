use std::sync::Arc;

use tauri::Manager;

use crate::config::AppConfig;
use crate::ws_bridge::WsBroadcaster;

use super::http::start_ws_server;
use super::{StartServerResult, WsServerHandle, WsServerState};

#[tauri::command]
pub async fn start_server(
    root_path: String,
    bind_ip: String,
    app: tauri::AppHandle,
    handle: tauri::State<'_, WsServerHandle>,
    config_state: tauri::State<'_, Arc<AppConfig>>,
    broadcaster: tauri::State<'_, Arc<WsBroadcaster>>,
    pty_manager: tauri::State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<StartServerResult, String> {
    {
        let running = handle.running.lock();
        if *running {
            return Err("サーバーは既に起動しています".to_string());
        }
    }

    let mut cfg = config_state.get_config()?;

    let detected = crate::vpn_detect::detect_all_interfaces();
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
        let (cert_path, key_path) = crate::tls::ensure_self_signed_cert(bind_ip_addr, &data_dir)?;
        cfg.server.tls.cert = cert_path.to_string_lossy().to_string();
        cfg.server.tls.key = key_path.to_string_lossy().to_string();
    }
    cfg.server.tls.enabled = true;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let remote_dir = if cfg!(debug_assertions) {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
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
            .map(|d| d.join("resources").join("remote"))
    };
    let server_state = Arc::new(WsServerState::new(
        remote_dir,
        Arc::clone(&broadcaster),
        Some(Arc::clone(&pty_manager)),
        Some(root_path),
        Arc::clone(config_state.inner()),
        Some(app.clone()),
        cfg.server.tls.enabled,
    ));

    start_ws_server(&cfg, server_state, shutdown_rx).await?;

    {
        let mut running = handle.running.lock();
        *running = true;
        let mut tx = handle.shutdown_tx.lock();
        *tx = Some(shutdown_tx);
        handle.active_bind.lock().replace(bind_ip.clone());
        *handle.tls_enabled.lock() = cfg.server.tls.enabled;
        handle.connection_mode.lock().replace(mode.clone());
    }

    Ok(StartServerResult { ip: bind_ip, mode })
}

#[tauri::command]
pub fn stop_server(handle: tauri::State<'_, WsServerHandle>) -> Result<(), String> {
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
        Ok(())
    } else {
        Err("サーバーは起動していません".to_string())
    }
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
pub fn broadcast_comments(
    comments: crate::protocol::CommentSync,
    broadcaster: tauri::State<'_, Arc<WsBroadcaster>>,
) {
    broadcaster.try_send(crate::protocol::WsMessage::CommentsSync(comments));
}
