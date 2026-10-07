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
    let presenter = app.state::<TauriDesktopLifecycle<R>>();
    match app
        .state::<Arc<DesktopLifecycleUsecase>>()
        .initialize()
        .await
    {
        Ok(()) => {
            presenter.connected_window(app.state::<Arc<DesktopLifecycleUsecase>>().connected(
                hidden,
                presenter.failure_window(),
                true,
            ))
        }
        Err(error) => presenter.connection_failed(error),
    }
}
pub(crate) fn settings_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: releashd::desktop_api::DesktopSettingsDto,
) {
    let applied = app
        .state::<LogFailure<Arc<DesktopLifecycleUsecase>>>()
        .call(|lifecycle| lifecycle.settings_changed(settings));
    app.state::<TauriDesktopLifecycle<R>>()
        .apply_settings(applied.unwrap_or(settings));
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
        Output = Result<(), crate::domain::daemon_connection::DaemonConnectionState>,
    >,
>(
    confirmed: Result<bool, String>,
    stop: impl FnOnce() -> F,
) -> Result<(), crate::domain::daemon_connection::DaemonConnectionState> {
    if confirmed
        .map_err(crate::domain::daemon_connection::DaemonConnectionState::TechnicalFailure)?
    {
        stop().await?;
    }
    Ok(())
}
#[cfg(test)]
#[path = "desktop_lifecycle_test.rs"]
mod desktop_lifecycle_tests;
