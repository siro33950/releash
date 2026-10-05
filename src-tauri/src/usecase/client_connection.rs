#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConnectionDto {
    pub url: String,
    pub token: String,
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ClientConnectionError(pub String);
