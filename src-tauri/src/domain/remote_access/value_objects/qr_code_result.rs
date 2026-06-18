use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct QrCodeResult {
    pub url: String,
    pub svg: String,
    pub token_svg: String,
}
