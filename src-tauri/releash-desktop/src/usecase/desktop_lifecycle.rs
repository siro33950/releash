use super::{
    daemon_connection::DaemonConnectionUsecase,
    login_item::{LoginItemError, LoginItemUsecase},
};
use crate::domain::{
    daemon_connection::{DaemonConnectionState, DaemonEndpoint},
    desktop_lifecycle::{self, ConnectedWindow, DesktopWindow},
};
use releashd::desktop_api::DesktopSettingsDto;
use std::sync::Arc;

pub struct DesktopLifecycleUsecase {
    connection: Arc<DaemonConnectionUsecase>,
    login: Arc<LoginItemUsecase>,
}
impl DesktopLifecycleUsecase {
    pub fn new(connection: Arc<DaemonConnectionUsecase>, login: Arc<LoginItemUsecase>) -> Self {
        Self { connection, login }
    }
    pub async fn initialize(&self) -> Result<(), DaemonConnectionState> {
        self.connection.connect().await
    }
    pub async fn endpoint(&self) -> Result<(DaemonEndpoint, bool), DaemonConnectionState> {
        self.connection.endpoint().await
    }
    pub async fn start(&self) -> Result<(), DaemonConnectionState> {
        self.initialize().await
    }
    pub async fn replace(&self) -> Result<(), DaemonConnectionState> {
        self.connection.replace().await
    }
    pub fn connected(
        &self,
        hidden: bool,
        failure_window: bool,
        changed: bool,
    ) -> Option<ConnectedWindow> {
        if !changed && !failure_window {
            return None;
        }
        self.connection.settings().map(|settings| {
            desktop_lifecycle::show_after_connection(
                hidden,
                settings.start_minimized,
                failure_window,
            )
        })
    }
    pub fn settings_changed(
        &self,
        settings: DesktopSettingsDto,
    ) -> Result<DesktopSettingsDto, LoginItemError> {
        self.login.restore(settings.auto_launch)?;
        Ok(settings)
    }
    pub async fn stop(&self) -> Result<(), DaemonConnectionState> {
        self.connection.stop().await
    }
    pub fn show(&self) -> DesktopWindow {
        desktop_lifecycle::window(self.connection.settings().is_some())
    }
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
