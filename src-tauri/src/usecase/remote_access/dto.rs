use crate::domain::remote_access::{DetectedInterface, QrCodeResult};

#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedInterfaceDto {
    pub name: String,
    pub ip: String,
    pub kind: String,
}

impl From<DetectedInterface> for DetectedInterfaceDto {
    fn from(interface: DetectedInterface) -> Self {
        Self {
            name: interface.name,
            ip: interface.ip,
            kind: interface.kind,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QrCodeResultDto {
    pub url: String,
    pub svg: String,
    pub token_svg: String,
}

impl From<QrCodeResult> for QrCodeResultDto {
    fn from(result: QrCodeResult) -> Self {
        Self {
            url: result.url,
            svg: result.svg,
            token_svg: result.token_svg,
        }
    }
}
