use crate::domain::{
    daemon_connection::DaemonConnectionFailure,
    desktop_lifecycle::{ConnectedWindow, DesktopWindow},
};
use crate::infrastructure::platform::{
    desktop_runtime::{self, ConfirmationReply, DesktopRuntime},
    window_lifecycle::{WindowPreferences, NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL},
};
use releashd::desktop_api::DesktopSettingsDto;
use std::{future::Future, pin::Pin, sync::Arc};

pub struct TauriDesktopLifecycle<R: tauri::Runtime>(pub Arc<DesktopRuntime<R>>);
impl<R: tauri::Runtime> TauriDesktopLifecycle<R> {
    pub fn apply_settings(&self, settings: DesktopSettingsDto) {
        self.0.apply_settings(
            preferences(settings),
            settings.crash_reporting,
            settings.performance_telemetry,
        );
    }
    pub fn failure_window(&self) -> bool {
        self.0.has_window(STARTUP_FAILURE_WINDOW_LABEL)
    }
    pub fn show(&self, window: DesktopWindow) -> Result<(), String> {
        let (label, install_drop) = match window {
            DesktopWindow::Normal => (NORMAL_WINDOW_LABEL, true),
            DesktopWindow::ConnectionFailure => (STARTUP_FAILURE_WINDOW_LABEL, false),
        };
        self.0.show(label, install_drop)
    }
    pub fn connected(&self, connected: crate::usecase::desktop_lifecycle::ConnectedDesktop) {
        if let Some(settings) = connected.settings {
            self.apply_settings(settings);
        }
        self.connected_window(connected.window);
    }
    pub fn connected_window(&self, window: Option<ConnectedWindow>) {
        if let Some(window) = window {
            self.0.connected_window(
                window == ConnectedWindow::Visible,
                NORMAL_WINDOW_LABEL,
                STARTUP_FAILURE_WINDOW_LABEL,
            );
        }
    }
    pub fn connection_failed(&self, failure: DaemonConnectionFailure) {
        self.0.connection_failed(
            super::daemon_connection::message(failure),
            STARTUP_FAILURE_WINDOW_LABEL,
        );
    }
    pub fn confirm_stop(&self) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send + '_>> {
        desktop_runtime::confirmation(|reply| {
            confirm_stop_with(
                |message, reply| self.0.confirm("サーバを停止", message, reply),
                reply,
            );
        })
    }
    pub fn stop_failed(&self, failure: DaemonConnectionFailure) {
        self.0.show_error(
            "サーバを停止できませんでした",
            super::daemon_connection::message(failure),
        );
    }
}
fn preferences(settings: DesktopSettingsDto) -> WindowPreferences {
    WindowPreferences {
        close_to_tray: settings.close_to_tray,
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn apply_desktop_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: DesktopSettingsDto,
) {
    desktop_runtime::apply_preferences(
        app,
        preferences(settings),
        settings.crash_reporting,
        settings.performance_telemetry,
    );
}
fn confirm_stop_with(
    confirm: impl FnOnce(&str, ConfirmationReply),
    reply: impl FnOnce(bool) + Send + 'static,
) {
    confirm(
        "サーバを停止すると、動いている agent の Session も停止します。停止しますか？",
        Box::new(reply),
    );
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
