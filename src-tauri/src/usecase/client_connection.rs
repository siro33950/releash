#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientConnectionDto {
    pub(crate) url: String,
    pub(crate) token: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ClientConnectionError(pub(crate) String);

pub(crate) trait ClientConnectionQueryService: Send + Sync {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError>;
}
