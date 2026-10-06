use super::daemon_supervision::DaemonSupervisionUsecase;
use std::sync::Arc;

#[derive(Clone, serde::Serialize)]
pub(crate) struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[async_trait::async_trait]
pub(crate) trait DesktopUpdateGateway:
    crate::domain::daemon_supervision::DesktopUpdateInstaller
{
    async fn check(&self) -> Result<Option<UpdateInfo>, String>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DesktopUpdateError {
    #[error("{0}")]
    Operation(String),
    #[error(transparent)]
    Supervision(#[from] super::daemon_supervision::DaemonSupervisionError),
}

pub(crate) struct DesktopUpdateUsecase {
    gateway: Arc<dyn DesktopUpdateGateway>,
    supervisor: Arc<DaemonSupervisionUsecase>,
    applying: tokio::sync::Mutex<()>,
}

impl DesktopUpdateUsecase {
    pub fn new(
        gateway: Arc<dyn DesktopUpdateGateway>,
        supervisor: Arc<DaemonSupervisionUsecase>,
    ) -> Self {
        Self {
            gateway,
            supervisor,
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
        self.supervisor.wait_for_update_stop().await?;
        self.supervisor.begin_update_install()?;
        if let Err(error) = self.gateway.install().await {
            self.supervisor.finish_update_install(Some(error.clone()));
            return Err(DesktopUpdateError::Operation(error));
        }
        if !self.supervisor.finish_update_install(None) {
            return Ok(());
        }
        if let Err(error) = self.gateway.restart() {
            self.supervisor.restart_failed(error.clone())?;
            return Err(DesktopUpdateError::Operation(error));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "desktop_update_test.rs"]
mod desktop_update_tests;
