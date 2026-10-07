use super::window_lifecycle::{WindowPreferences, WindowPreferencesState};
use releashd::desktop_api::{TelemetryGuard, TelemetryPort};
use std::{future::Future, pin::Pin, sync::OnceLock};
use tauri::Manager;

pub struct DesktopRuntime<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    telemetry: OnceLock<Option<TelemetryGuard>>,
    #[cfg(test)]
    telemetry_initializations: std::sync::atomic::AtomicUsize,
}
impl<R: tauri::Runtime> DesktopRuntime<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            app,
            telemetry: OnceLock::new(),
            #[cfg(test)]
            telemetry_initializations: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    pub fn apply_settings(
        &self,
        preferences: WindowPreferences,
        crash_reporting: bool,
        performance_telemetry: bool,
    ) {
        self.telemetry.get_or_init(|| {
            #[cfg(test)]
            self.telemetry_initializations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            releashd::desktop_api::init_telemetry(crash_reporting, performance_telemetry)
        });
        apply_preferences(
            &self.app,
            preferences,
            crash_reporting,
            performance_telemetry,
        );
    }
    pub fn has_window(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }
    pub fn show(&self, label: &str, install_drop: bool) -> Result<(), String> {
        show_window(&self.app, label, install_drop)
    }
    pub fn connected_window(
        &self,
        visible: bool,
        normal_label: &'static str,
        failure_label: &'static str,
    ) {
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            if visible {
                if let Err(error) = show_window(&app, normal_label, true) {
                    log::error!("{error}");
                }
            }
            if let Some(failed) = app.get_webview_window(failure_label) {
                let _ = failed.destroy();
            }
        });
    }
    pub fn connection_failed(&self, message: String, label: &'static str) {
        log::error!("{message}");
        let app = self.app.clone();
        let _ = self.app.run_on_main_thread(move || {
            if let Err(error) = show_window(&app, label, false) {
                log::error!("{error}");
            }
        });
    }
    pub fn confirm(&self, title: &str, message: &str, reply: ConfirmationReply) {
        use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
        self.app
            .dialog()
            .message(message)
            .title(title)
            .buttons(MessageDialogButtons::OkCancel)
            .show(reply);
    }
    pub fn show_error(&self, title: &str, message: String) {
        use tauri_plugin_dialog::DialogExt;
        self.app.dialog().message(message).title(title).show(|_| {});
    }
}

pub fn apply_preferences<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    preferences: WindowPreferences,
    crash_reporting: bool,
    performance_telemetry: bool,
) {
    if let Some(state) = app.try_state::<WindowPreferencesState>() {
        *state.0.write() = preferences;
    } else {
        app.manage(WindowPreferencesState(parking_lot::RwLock::new(
            preferences,
        )));
    }
    releashd::desktop_api::TelemetryGateway.set_crash_reporting_enabled(crash_reporting);
    releashd::desktop_api::TelemetryGateway.set_performance_enabled(performance_telemetry);
}

pub type ConfirmationReply = Box<dyn FnOnce(bool) + Send>;
pub fn confirmation(
    confirm: impl FnOnce(ConfirmationReply),
) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send>> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    confirm(Box::new(move |confirmed| {
        let _ = sender.send(confirmed);
    }));
    Box::pin(async { receiver.await.map_err(|e| e.to_string()) })
}

pub fn record_window_ready() {
    releashd::desktop_api::record_startup_from_origin(
        releashd::desktop_api::Startup::FirstWindowReady,
    );
    releashd::desktop_api::record_startup_from_origin(releashd::desktop_api::Startup::AppStartup);
}

fn show_window<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &str,
    install_drop: bool,
) -> Result<(), String> {
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => {
            let mut config = app
                .config()
                .app
                .windows
                .first()
                .cloned()
                .ok_or("Window configuration is missing")?;
            config.label = label.into();
            config.create = true;
            let window = tauri::WebviewWindowBuilder::from_config(app, &config)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            if install_drop {
                crate::infrastructure::platform::native_drop::install(&window);
            }
            if install_drop {
                record_window_ready();
            }
            window
        }
    };
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "desktop_runtime_test.rs"]
mod desktop_runtime_tests;
