use std::sync::Arc;

#[derive(Clone, serde::Serialize)]
pub(crate) struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[async_trait::async_trait]
pub(crate) trait DesktopUpdateGateway: DesktopUpdateInstaller {
    async fn check(&self) -> Result<Option<UpdateInfo>, String>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DesktopUpdateError {
    #[error("{0}")]
    Operation(String),
}

pub(crate) struct DesktopUpdateUsecase {
    gateway: Arc<dyn DesktopUpdateGateway>,
    applying: tokio::sync::Mutex<()>,
}

impl DesktopUpdateUsecase {
    pub fn new(gateway: Arc<dyn DesktopUpdateGateway>) -> Self {
        Self {
            gateway,
            applying: tokio::sync::Mutex::new(()),
        }
    }
    pub async fn check(&self) -> Result<Option<UpdateInfo>, DesktopUpdateError> {
        self.gateway
            .check()
            .await
            .map_err(DesktopUpdateError::Operation)
    }
    pub async fn apply(&self) -> Result<(), DesktopUpdateError> {
        let _guard = self.applying.try_lock().map_err(|_| {
            DesktopUpdateError::Operation("An update is already in progress.".into())
        })?;
        self.gateway
            .download()
            .await
            .map_err(DesktopUpdateError::Operation)?;
        self.gateway
            .install()
            .await
            .map_err(DesktopUpdateError::Operation)?;
        self.gateway
            .restart()
            .map_err(DesktopUpdateError::Operation)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "desktop_update_test.rs"]
mod desktop_update_tests;

#[async_trait::async_trait]
pub(crate) trait DesktopUpdateInstaller: Send + Sync {
    async fn download(&self) -> Result<(), String>;
    async fn install(&self) -> Result<(), String>;
    fn restart(&self) -> Result<(), String>;
}
