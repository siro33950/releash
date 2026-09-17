use std::sync::Arc;

pub(crate) trait CliInstallGateway: Send + Sync {
    fn install(&self) -> Result<String, String>;
}
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CliInstallError(String);
pub(crate) struct CliInstallUsecase(pub Arc<dyn CliInstallGateway>);
impl CliInstallUsecase {
    pub async fn install(&self) -> Result<String, CliInstallError> {
        let gateway = self.0.clone();
        tokio::task::spawn_blocking(move || gateway.install())
            .await
            .map_err(|e| CliInstallError(e.to_string()))?
            .map_err(CliInstallError)
    }
}
#[cfg(test)]
#[path = "cli_install_test.rs"]
mod cli_install_tests;
