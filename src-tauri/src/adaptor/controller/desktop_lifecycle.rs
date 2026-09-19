use crate::infrastructure::platform::window_lifecycle::{
    NORMAL_WINDOW_LABEL, STARTUP_FAILURE_WINDOW_LABEL,
};
use crate::{
    domain::daemon_supervision::StopIntent, usecase::daemon_supervision::DaemonSupervisionUsecase,
};
use std::sync::Arc;
use tauri::Manager;

pub(crate) fn show(app: &tauri::AppHandle) -> Result<(), String> {
    let supervisor = app.state::<Arc<DaemonSupervisionUsecase>>();
    let ready = supervisor.connection().is_ok();
    let label = if ready {
        NORMAL_WINDOW_LABEL
    } else {
        STARTUP_FAILURE_WINDOW_LABEL
    };
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
            if ready {
                crate::infrastructure::platform::native_drop::install(&window);
            }
            if ready {
                crate::desktop::record_window_ready();
            }
            window
        }
    };
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

pub(crate) fn request_quit(supervisor: &DaemonSupervisionUsecase) {
    if let Err(error) = supervisor.stop(StopIntent::Quit(0)) {
        log::error!("{error}");
    }
}

trait DesktopHost: Send + Sync + 'static {
    fn has_failure_window(&self) -> bool;
    fn ready(
        &self,
        settings: crate::usecase::app_config::query_service::DesktopSettingsDto,
        first: bool,
        show: bool,
    );
    fn show(&self);
    fn exit(&self, code: i32);
    fn restart(&self) -> Result<(), String>;
}

struct TauriDesktop(tauri::AppHandle);
impl DesktopHost for TauriDesktop {
    fn has_failure_window(&self) -> bool {
        self.0
            .get_webview_window(STARTUP_FAILURE_WINDOW_LABEL)
            .is_some()
    }
    fn ready(
        &self,
        settings: crate::usecase::app_config::query_service::DesktopSettingsDto,
        first: bool,
        show_window: bool,
    ) {
        if first {
            self.0
                .manage(crate::infrastructure::telemetry::init_telemetry(
                    settings.crash_reporting,
                    settings.performance_telemetry,
                ));
        }
        crate::desktop::apply_desktop_settings(&self.0, settings);
        let handle = self.0.clone();
        let _ = self.0.run_on_main_thread(move || {
            if let Some(failure) = handle.get_webview_window(STARTUP_FAILURE_WINDOW_LABEL) {
                let _ = failure.destroy();
            }
            if show_window {
                if let Err(error) = show(&handle) {
                    log::error!("{error}");
                }
            }
        });
    }
    fn show(&self) {
        let handle = self.0.clone();
        let _ = self.0.run_on_main_thread(move || {
            if let Err(error) = show(&handle) {
                log::error!("{error}");
            }
        });
    }
    fn exit(&self, code: i32) {
        crate::infrastructure::platform::native_termination::exit(&self.0, code);
    }
    fn restart(&self) -> Result<(), String> {
        crate::infrastructure::platform::desktop_restart::restart(&self.0)
    }
}

pub(crate) fn observe(
    app: tauri::AppHandle,
    supervisor: Arc<DaemonSupervisionUsecase>,
    hidden: bool,
) {
    tauri::async_runtime::spawn(observe_with(TauriDesktop(app), supervisor, hidden));
}

pub(crate) fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    crate::infrastructure::platform::window_lifecycle::handle_run_event(
        app,
        event,
        |code| {
            app.state::<Arc<super::application_lifecycle::ApplicationQuitIngress>>()
                .request(crate::usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code })
        },
        || {
            if let Err(error) = show(app) {
                log::error!("{error}");
            }
        },
    );
}

async fn observe_with(
    host: impl DesktopHost,
    supervisor: Arc<DaemonSupervisionUsecase>,
    hidden: bool,
) {
    let mut changes = supervisor.subscribe();
    let mut first_ready = true;
    loop {
        changes.borrow_and_update();
        use crate::domain::daemon_supervision::DesktopAction;
        match supervisor.desktop_action(hidden, first_ready, host.has_failure_window()) {
            DesktopAction::Ready { show_window } => {
                if let Ok(connection) = supervisor.connection() {
                    host.ready(connection.settings, first_ready, show_window);
                    first_ready = false;
                }
            }
            DesktopAction::Show => host.show(),
            DesktopAction::Exit(code) => {
                host.exit(code);
                break;
            }
            DesktopAction::Restart => match host.restart() {
                Ok(()) => break,
                Err(error) => {
                    let _ = supervisor.restart_failed(error);
                }
            },
            DesktopAction::Wait => {}
        }
        if changes.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
