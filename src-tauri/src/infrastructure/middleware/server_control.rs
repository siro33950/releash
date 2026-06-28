use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster;
use crate::domain::app_config::ConfigRepository;
use crate::usecase::agent_session::session::AgentStreamResyncReadModel;
use crate::usecase::pty_session::query_service::PtySessionReplayReader;

use super::http_upgrade::start_ws_server;
use super::{StartServerResult, WsServerHandle, WsServerState};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub struct ServerStatusPayload {
    pub running: bool,
    pub bound_ip: Option<String>,
    pub connection_mode: Option<String>,
}

#[allow(dead_code)]
pub async fn start_server_core(
    app: &tauri::AppHandle,
    bind_ip: String,
) -> Result<StartServerResult, String> {
    let handle = app.state::<WsServerHandle>();
    let config_state = app.state::<Arc<dyn ConfigRepository>>();
    let broadcaster = app.state::<Arc<WsBroadcaster>>();
    let pty_replay_reader = app.state::<Arc<dyn PtySessionReplayReader>>();
    let stream_resync_read_model = app.state::<Arc<dyn AgentStreamResyncReadModel>>();

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

    let server_state = Arc::new(WsServerState::new(
        Arc::clone(&broadcaster),
        Arc::clone(&pty_replay_reader),
        Arc::clone(config_state.inner()),
        Arc::clone(stream_resync_read_model.inner()),
        cfg.server.tls.enabled,
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

    Ok(StartServerResult { ip: bind_ip, mode })
}

#[allow(dead_code)]
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
