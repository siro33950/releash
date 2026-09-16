#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientConnectionDto {
    pub(crate) url: String,
    pub(crate) auth_subprotocol: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ClientConnectionError(pub(crate) String);

#[async_trait::async_trait]
pub(crate) trait ClientConnectionQueryService: Send + Sync {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError>;
    #[cfg(feature = "desktop")]
    async fn desktop_settings(
        &self,
    ) -> Result<super::app_config::query_service::DesktopSettingsDto, ClientConnectionError>;
}

pub(crate) struct ClientConnectionUsecase(pub(crate) Box<dyn ClientConnectionQueryService>);

impl ClientConnectionUsecase {
    #[cfg(feature = "desktop")]
    pub(crate) async fn desktop_settings(
        &self,
    ) -> Result<super::app_config::query_service::DesktopSettingsDto, ClientConnectionError> {
        self.0.desktop_settings().await
    }

    pub(crate) fn endpoint(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        self.0.read()
    }
}

#[cfg(test)]
#[path = "client_connection_test.rs"]
mod client_connection_tests;
