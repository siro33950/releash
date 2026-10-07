use crate::adaptor::presenter::daemon_connection::{self, DesktopConnectionFailure};
use crate::usecase::daemon_connection::DaemonConnectionUsecase;
use crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase;
use std::sync::Arc;
pub(crate) const COMMAND_NAMES: &[&str] = &[
    "get_desktop_connection_failure",
    "start_daemon",
    "replace_daemon",
    "quit_desktop",
    "get_login_item_status",
    "open_login_item_settings",
    "install_cli",
    "set_login_item_enabled",
    "check_desktop_update",
    "install_desktop_update",
];
pub(crate) fn register<R: tauri::Runtime>(
    router: &mut super::CommandRouter<super::InvokeHandler<R>>,
) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![
            get_desktop_connection_failure,
            start_daemon,
            replace_daemon,
            quit_desktop,
            get_login_item_status,
            open_login_item_settings,
            install_cli,
            set_login_item_enabled,
            check_desktop_update,
            install_desktop_update
        ]),
    );
}
#[tauri::command]
fn get_desktop_connection_failure(
    connection: tauri::State<'_, Arc<DaemonConnectionUsecase>>,
) -> Option<DesktopConnectionFailure> {
    connection.failure().map(daemon_connection::failure)
}
#[tauri::command]
async fn start_daemon<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    connection: tauri::State<'_, Arc<DesktopLifecycleUsecase>>,
) -> Result<(), String> {
    use tauri::Manager;
    app.state::<crate::common::serial::Serial>()
        .call(async {
            let presenter = app
                .state::<crate::adaptor::presenter::desktop_lifecycle::TauriDesktopLifecycle<R>>();
            let connected = connection
                .initialize(None, presenter.failure_window())
                .await
                .map_err(crate::adaptor::presenter::daemon_connection::message)?;
            super::super::desktop_lifecycle::connected(&app, connected);
            Ok(())
        })
        .await
}
#[tauri::command]
async fn replace_daemon<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    connection: tauri::State<'_, Arc<DesktopLifecycleUsecase>>,
) -> Result<(), String> {
    use tauri::Manager;
    app.state::<crate::common::serial::Serial>()
        .call(async {
            let presenter = app
                .state::<crate::adaptor::presenter::desktop_lifecycle::TauriDesktopLifecycle<R>>();
            let connected = connection
                .replace(presenter.failure_window())
                .await
                .map_err(crate::adaptor::presenter::daemon_connection::message)?;
            super::super::desktop_lifecycle::connected(&app, connected);
            Ok(())
        })
        .await
}
#[tauri::command]
pub(crate) fn quit_desktop<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    super::super::desktop_lifecycle::quit(&app, 0);
}
#[tauri::command]
async fn get_login_item_status(
    login: tauri::State<'_, Arc<crate::usecase::login_item::LoginItemUsecase>>,
) -> Result<crate::usecase::login_item::LoginItemState, String> {
    login.status().await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn set_login_item_enabled(
    login: tauri::State<'_, Arc<crate::usecase::login_item::LoginItemUsecase>>,
    enabled: bool,
) -> Result<crate::usecase::login_item::LoginItemState, String> {
    login.set_enabled(enabled).await.map_err(|e| e.to_string())
}
#[tauri::command]
fn open_login_item_settings(
    login: tauri::State<'_, Arc<crate::usecase::login_item::LoginItemUsecase>>,
) -> Result<(), String> {
    login.open_settings().map_err(|e| e.to_string())
}
#[tauri::command]
async fn install_cli(
    installer: tauri::State<'_, crate::usecase::cli_install::CliInstallUsecase>,
) -> Result<String, String> {
    installer.install().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_desktop_update(
    update: tauri::State<'_, crate::usecase::desktop_update::DesktopUpdateUsecase>,
) -> Result<Option<crate::usecase::desktop_update::UpdateInfo>, String> {
    update.check().await.map_err(|error| error.to_string())
}
#[tauri::command]
async fn install_desktop_update(
    update: tauri::State<'_, crate::usecase::desktop_update::DesktopUpdateUsecase>,
) -> Result<(), String> {
    update.apply().await.map_err(|error| error.to_string())
}
