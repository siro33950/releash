use super::{
    daemon_connection::DaemonConnectionUsecase,
    login_item::{LoginItemError, LoginItemUsecase},
};
use crate::domain::{
    daemon_connection::{DaemonConnectionFailure, DaemonEndpoint},
    desktop_lifecycle::{ConnectedWindow, DesktopLifecycle, DesktopWindow},
};
use releashd::desktop_api::DesktopSettingsDto;
use std::sync::Arc;

pub struct ConnectedDesktop {
    pub endpoint: DaemonEndpoint,
    pub settings: Option<DesktopSettingsDto>,
    pub window: Option<ConnectedWindow>,
    pub restoration: Result<(), LoginItemError>,
}

pub struct DesktopLifecycleUsecase {
    connection: Arc<DaemonConnectionUsecase>,
    login: Arc<LoginItemUsecase>,
    window: parking_lot::Mutex<DesktopLifecycle>,
}
impl DesktopLifecycleUsecase {
    pub fn new(connection: Arc<DaemonConnectionUsecase>, login: Arc<LoginItemUsecase>) -> Self {
        Self {
            connection,
            login,
            window: parking_lot::Mutex::new(DesktopLifecycle::default()),
        }
    }
    pub async fn initialize(
        &self,
        hidden: Option<bool>,
        failure_window: bool,
    ) -> Result<ConnectedDesktop, DaemonConnectionFailure> {
        let (endpoint, changed) = self.connection.connect().await?;
        Ok(self.connected(endpoint, hidden.unwrap_or(false), failure_window, changed))
    }
    pub async fn endpoint(
        &self,
        failure_window: bool,
    ) -> Result<ConnectedDesktop, DaemonConnectionFailure> {
        let (endpoint, changed) = self.connection.endpoint().await?;
        Ok(self.connected(endpoint, false, failure_window, changed))
    }
    pub async fn replace(
        &self,
        failure_window: bool,
    ) -> Result<ConnectedDesktop, DaemonConnectionFailure> {
        let (endpoint, changed) = self.connection.replace().await?;
        Ok(self.connected(endpoint, false, failure_window, changed))
    }
    fn connected(
        &self,
        endpoint: DaemonEndpoint,
        hidden: bool,
        failure_window: bool,
        changed: bool,
    ) -> ConnectedDesktop {
        if changed {
            let settings = self.connection.initial_settings();
            ConnectedDesktop {
                restoration: settings
                    .and_then(|settings| self.settings_changed(&endpoint, settings))
                    .map(|(_, restoration)| restoration)
                    .unwrap_or(Ok(())),
                endpoint,
                settings,
                window: settings.map(|settings| {
                    self.window
                        .lock()
                        .connected(hidden, settings.start_minimized, failure_window)
                }),
            }
        } else {
            ConnectedDesktop {
                endpoint,
                restoration: Ok(()),
                settings: None,
                window: if failure_window {
                    self.connection.settings().map(|settings| {
                        self.window.lock().connected(
                            hidden,
                            settings.start_minimized,
                            failure_window,
                        )
                    })
                } else {
                    None
                },
            }
        }
    }

    pub fn settings_changed(
        &self,
        endpoint: &DaemonEndpoint,
        settings: DesktopSettingsDto,
    ) -> Option<(DesktopSettingsDto, Result<(), LoginItemError>)> {
        if !self.connection.is_connected_to(endpoint) {
            return None;
        }
        Some((settings, self.login.restore(settings.auto_launch)))
    }
    pub async fn stop(&self) -> Result<(), DaemonConnectionFailure> {
        self.connection.stop().await
    }
    pub fn show(&self) -> DesktopWindow {
        self.window
            .lock()
            .show(self.connection.settings().is_some())
    }
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
