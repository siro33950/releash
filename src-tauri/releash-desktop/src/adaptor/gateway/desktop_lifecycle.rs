use crate::infrastructure::platform::{
    desktop_runtime::{self, ConfirmationReply, DesktopRuntime},
    window_lifecycle::{WindowPreferences, NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL},
};
use crate::usecase::desktop_lifecycle::DesktopLifecycleOutput;
use releashd::desktop_api::DesktopSettingsDto;
use std::{future::Future, pin::Pin, sync::Arc};

pub struct TauriDesktopLifecycle<R: tauri::Runtime>(pub Arc<DesktopRuntime<R>>);
impl<R: tauri::Runtime> DesktopLifecycleOutput for TauriDesktopLifecycle<R> {
    fn apply_settings(&self, settings: DesktopSettingsDto) {
        self.0.apply_settings(
            preferences(settings),
            settings.crash_reporting,
            settings.performance_telemetry,
        );
    }
    fn failure_window(&self) -> bool {
        self.0.has_window(STARTUP_FAILURE_WINDOW_LABEL)
    }
    fn show(&self, ready: bool) -> Result<(), String> {
        self.0.show(
            if ready {
                NORMAL_WINDOW_LABEL
            } else {
                STARTUP_FAILURE_WINDOW_LABEL
            },
            ready,
        )
    }
    fn connected_window(&self, visible: bool) {
        self.0
            .connected_window(visible, NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL);
    }
    fn connection_failed(&self, message: String) {
        self.0
            .connection_failed(message, STARTUP_FAILURE_WINDOW_LABEL);
    }
    fn confirm_stop(&self) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send + '_>> {
        desktop_runtime::confirmation(|reply| {
            confirm_stop_with(
                |message, reply| self.0.confirm("サーバを停止", message, reply),
                reply,
            );
        })
    }
    fn stop_failed(&self, message: String) {
        self.0.show_error("サーバを停止できませんでした", message);
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
