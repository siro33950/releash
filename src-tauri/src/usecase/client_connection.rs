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

pub(crate) struct ClientConnectionUsecase(pub(crate) Box<dyn ClientConnectionQueryService>);

impl ClientConnectionUsecase {
    pub(crate) fn endpoint(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        self.0.read()
    }
}

#[cfg(test)]
#[path = "client_connection_test.rs"]
mod client_connection_tests;
