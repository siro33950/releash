use crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase;
use std::sync::Arc;
use tauri::Manager;
pub(crate) fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    app.state::<Arc<DesktopLifecycleUsecase>>().show()
}
pub(crate) async fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>, hidden: bool) {
    let lifecycle = app.state::<Arc<DesktopLifecycleUsecase>>();
    if let Err(error) = lifecycle.initialize(hidden).await {
        lifecycle.connection_failed(crate::adaptor::presenter::daemon_connection::message(error));
    }
}
pub(crate) fn settings_changed<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    settings: releashd::desktop_api::DesktopSettingsDto,
) {
    app.state::<Arc<DesktopLifecycleUsecase>>()
        .settings_changed(settings);
}
pub(crate) fn confirm_stop(app: tauri::AppHandle) {
    let lifecycle = app.state::<Arc<DesktopLifecycleUsecase>>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = lifecycle.confirm_stop().await {
            lifecycle.stop_failed(crate::adaptor::presenter::daemon_connection::message(error));
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
