use crate::domain::remote_access::services::build_connection_url;
use crate::domain::remote_access::{QrCodeResult, QrRenderGateway, RemoteAccessError};

pub fn get_connection_qr(
    qr: &dyn QrRenderGateway,
    bind: &str,
    port: u16,
    token: &str,
    tls_enabled: bool,
) -> Result<QrCodeResult, RemoteAccessError> {
    let url = build_connection_url(bind, port, tls_enabled);
    let svg = qr.generate_qr_svg(&url)?;
    let token_svg = qr.generate_qr_svg(token)?;
    Ok(QrCodeResult {
        url,
        svg,
        token_svg,
    })
}
