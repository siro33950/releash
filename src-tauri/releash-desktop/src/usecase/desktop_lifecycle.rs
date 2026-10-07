use super::{daemon_connection::DaemonConnectionUsecase, login_item::LoginItemUsecase};
use crate::domain::daemon_connection::{DaemonConnection, DaemonConnectionState, DaemonEndpoint};
use releashd::desktop_api::DesktopSettingsDto;
use std::sync::Arc;

pub trait DesktopLifecycleOutput: Send + Sync {
    fn apply_settings(&self, settings: DesktopSettingsDto);
    fn failure_window(&self) -> bool;
    fn show(&self, ready: bool) -> Result<(), String>;
    fn connected_window(&self, visible: bool);
    fn connection_failed(&self, message: String);
    fn confirm_stop(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send + '_>>;
    fn stop_failed(&self, message: String);
}

pub struct DesktopLifecycleUsecase {
    connection: Arc<DaemonConnectionUsecase>,
    login: Arc<LoginItemUsecase>,
    output: Arc<dyn DesktopLifecycleOutput>,
}
impl DesktopLifecycleUsecase {
    pub fn new(
        connection: Arc<DaemonConnectionUsecase>,
        login: Arc<LoginItemUsecase>,
        output: Arc<dyn DesktopLifecycleOutput>,
    ) -> Self {
        Self {
            connection,
            login,
            output,
        }
    }
    pub async fn initialize(&self, hidden: bool) -> Result<(), DaemonConnectionState> {
        self.connection.connect().await?;
        self.connected(hidden);
        Ok(())
    }
    pub async fn endpoint(&self) -> Result<DaemonEndpoint, DaemonConnectionState> {
        let (endpoint, changed) = self.connection.endpoint().await?;
        if changed {
            self.connected(false);
        }
        Ok(endpoint)
    }
    pub async fn start(&self) -> Result<(), DaemonConnectionState> {
        self.initialize(false).await
    }
    pub async fn replace(&self) -> Result<(), DaemonConnectionState> {
        self.connection.replace().await?;
        self.connected(false);
        Ok(())
    }
    fn connected(&self, hidden: bool) {
        if let Some(settings) = self.connection.settings() {
            self.settings_changed(settings);
            self.output
                .connected_window(DaemonConnection::show_after_connection(
                    hidden,
                    settings.start_minimized,
                    self.output.failure_window(),
                ));
        }
    }
    pub fn settings_changed(&self, settings: DesktopSettingsDto) {
        self.output.apply_settings(settings);
        if let Err(error) = self.login.restore(settings.auto_launch) {
            log::error!("{error}");
        }
    }
    pub async fn confirm_stop(&self) -> Result<(), DaemonConnectionState> {
        if self
            .output
            .confirm_stop()
            .await
            .map_err(DaemonConnectionState::TechnicalFailure)?
        {
            self.connection.stop().await?;
        }
        Ok(())
    }
    pub fn stop_failed(&self, message: String) {
        self.output.stop_failed(message);
    }
    pub fn show(&self) -> Result<(), String> {
        self.output.show(self.connection.settings().is_some())
    }
    pub fn connection_failed(&self, message: String) {
        self.output.connection_failed(message);
    }
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
