use crate::adaptor::presenter::desktop_lifecycle::TauriDesktopLifecycle;
use crate::common::log_failure::LogFailure;
use crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase;
use std::sync::Arc;
use tauri::Manager;
pub(crate) fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    app.state::<TauriDesktopLifecycle<R>>()
        .show(app.state::<Arc<DesktopLifecycleUsecase>>().show())
}
pub(crate) async fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    app.state::<crate::common::serial::Serial>()
        .call(async {
            let presenter = app.state::<TauriDesktopLifecycle<R>>();
            match app
                .state::<Arc<DesktopLifecycleUsecase>>()
                .initialize(Some(hidden), presenter.failure_window())
                .await
            {
                Ok(value) => connected(app, value),
                Err(error) => presenter.connection_failed(error),
            }
        })
        .await;
}
pub(crate) fn connected<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    connected: crate::usecase::desktop_lifecycle::ConnectedDesktop,
) {
    app.state::<LogFailure<Arc<DesktopLifecycleUsecase>>>()
        .record(&connected.restoration);
    app.state::<TauriDesktopLifecycle<R>>().connected(connected);
}

pub(crate) async fn settings_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: releashd::desktop_api::DesktopSettingsDto,
) {
    app.state::<crate::common::serial::Serial>()
        .call(async {
            let _ = app
                .state::<LogFailure<Arc<DesktopLifecycleUsecase>>>()
                .call(|lifecycle| lifecycle.settings_changed(settings));
            app.state::<TauriDesktopLifecycle<R>>()
                .apply_settings(settings);
        })
        .await;
}

pub(crate) fn confirm_stop(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let presenter = app.state::<TauriDesktopLifecycle<tauri::Wry>>();
        let lifecycle = app.state::<Arc<DesktopLifecycleUsecase>>();
        let result =
            stop_after_confirmation(presenter.confirm_stop().await, || lifecycle.stop()).await;
        if let Err(error) = result {
            presenter.stop_failed(error);
        }
    });
}
pub(crate) fn quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>, code: i32) {
    crate::infrastructure::platform::native_termination::exit(app, code);
}
pub(crate) fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    crate::infrastructure::platform::window_lifecycle::handle_run_event(
        app,
        event,
        |code| quit(app, code),
        || {
            if let Err(error) = show(app) {
                log::error!("{error}");
            }
        },
    );
}

async fn stop_after_confirmation<
    F: std::future::Future<
        Output = Result<(), crate::domain::daemon_connection::DaemonConnectionFailure>,
    >,
>(
    confirmed: Result<bool, String>,
    stop: impl FnOnce() -> F,
) -> Result<(), crate::domain::daemon_connection::DaemonConnectionFailure> {
    if confirmed
        .map_err(crate::domain::daemon_connection::DaemonConnectionFailure::TechnicalFailure)?
    {
        stop().await?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
