use std::sync::Arc;

use serde::Serialize;

use crate::config::AppConfig;
use crate::vpn_detect::detect_vpn_ip;
use crate::ws_server::WsServerHandle;

#[derive(Debug, Clone, Serialize)]
pub struct QrCodeResult {
    pub url: String,
    pub svg: String,
    pub token_svg: String,
}

fn build_connection_url(bind: &str, port: u16, tls_enabled: bool) -> Result<String, String> {
    let scheme = if tls_enabled { "https" } else { "http" };

    let ip = match bind {
        "127.0.0.1" | "localhost" => "127.0.0.1".to_string(),
        "meshnet" | "mesh_vpn" => detect_vpn_ip()
            .map(|iface| iface.ip.to_string())
            .ok_or("メッシュVPNが見つかりません")?,
        "any" | "0.0.0.0" => detect_vpn_ip()
            .map(|iface| iface.ip.to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string()),
        addr => addr.to_string(),
    };

    Ok(format!("{scheme}://{ip}:{port}"))
}

fn generate_qr_svg(data: &str) -> Result<String, String> {
    use qrcode::render::svg;
    use qrcode::QrCode;

    let code = QrCode::new(data).map_err(|e| format!("QRコード生成失敗: {e}"))?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(200, 200)
        .dark_color(svg::Color("#e0e0e0"))
        .light_color(svg::Color("#1a1a1a"))
        .build();
    Ok(svg)
}

#[tauri::command]
pub fn get_connection_qr(
    state: tauri::State<'_, Arc<AppConfig>>,
    server_handle: tauri::State<'_, WsServerHandle>,
) -> Result<QrCodeResult, String> {
    let config = state.get_config()?;
    let bind = server_handle
        .active_bind()
        .unwrap_or_else(|| config.server.bind.clone());
    let tls_enabled = server_handle.is_tls_enabled();
    let url = build_connection_url(&bind, config.server.port, tls_enabled)?;
    let svg = generate_qr_svg(&url)?;
    let token_svg = generate_qr_svg(&config.server.token)?;
    Ok(QrCodeResult {
        url,
        svg,
        token_svg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_url_localhost() {
        let url = build_connection_url("127.0.0.1", 9700, false).unwrap();
        assert_eq!(url, "http://127.0.0.1:9700");
    }

    #[test]
    fn test_build_url_with_tls() {
        let url = build_connection_url("127.0.0.1", 9700, true).unwrap();
        assert!(url.starts_with("https://"));
        assert!(!url.contains("token"));
    }

    #[test]
    fn test_build_url_custom_ip() {
        let url = build_connection_url("192.168.1.100", 8080, false).unwrap();
        assert_eq!(url, "http://192.168.1.100:8080");
    }

    #[test]
    fn test_generate_qr_svg_valid() {
        let svg = generate_qr_svg("http://127.0.0.1:9700?token=test").unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn test_generate_qr_svg_has_dimensions() {
        let svg = generate_qr_svg("test").unwrap();
        assert!(svg.contains("width="));
        assert!(svg.contains("height="));
    }
}
