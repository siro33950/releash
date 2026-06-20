use std::sync::Arc;

use crate::adaptor::gateway::remote_access::{QrCodeRenderGateway, SystemNetworkInterfaceGateway};
use crate::domain::app_config::ConfigRepository;
use crate::usecase::remote_access::dto::{DetectedInterfaceDto, QrCodeResultDto};
use crate::ws_server::WsServerHandle;

#[tauri::command]
pub async fn get_network_info() -> Result<Vec<DetectedInterfaceDto>, String> {
    tokio::task::spawn_blocking(|| {
        crate::usecase::remote_access::network_usecase::get_network_info(
            &SystemNetworkInterfaceGateway,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    })
    .await
    .map_err(|e| format!("task join error: {e}"))
}

#[tauri::command]
pub async fn detect_vpn_tunnel() -> Result<Option<serde_json::Value>, String> {
    tokio::task::spawn_blocking(|| {
        crate::usecase::remote_access::network_usecase::detect_vpn_tunnel(
            &SystemNetworkInterfaceGateway,
        )
        .map(|iface| {
            serde_json::json!({
                "name": iface.name,
                "ip": iface.ip.to_string()
            })
        })
    })
    .await
    .map_err(|e| format!("task join error: {e}"))
}

#[tauri::command]
pub fn get_connection_qr(
    state: tauri::State<'_, Arc<dyn ConfigRepository>>,
    server_handle: tauri::State<'_, WsServerHandle>,
) -> Result<QrCodeResultDto, String> {
    let config = state.load().map_err(|e| e.to_string())?;
    let bind = server_handle
        .active_bind()
        .unwrap_or_else(|| config.server.bind.clone());
    crate::usecase::remote_access::qr_usecase::get_connection_qr(
        &QrCodeRenderGateway,
        &bind,
        config.server.port,
        &config.server.token,
        server_handle.is_tls_enabled(),
    )
    .map(Into::into)
    .map_err(|e| e.to_string())
}
