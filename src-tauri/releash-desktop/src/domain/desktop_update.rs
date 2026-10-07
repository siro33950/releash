#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DesktopUpdateFailure {
    #[error("{0}")]
    TechnicalFailure(String),
}

#[async_trait::async_trait]
pub(crate) trait DesktopUpdateInstaller: Send + Sync {
    async fn download(&self) -> Result<(), DesktopUpdateFailure>;
    async fn install(&self) -> Result<(), DesktopUpdateFailure>;
    fn restart(&self) -> Result<(), DesktopUpdateFailure>;
}
