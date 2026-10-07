#[async_trait::async_trait]
pub(crate) trait DesktopUpdateInstaller: Send + Sync {
    async fn download(&self) -> Result<(), String>;
    async fn install(&self) -> Result<(), String>;
    fn restart(&self) -> Result<(), String>;
}
