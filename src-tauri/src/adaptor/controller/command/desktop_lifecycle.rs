use crate::domain::daemon_supervision::StopIntent;
use crate::usecase::daemon_supervision::{DaemonStatus, DaemonSupervisionUsecase};
use std::sync::Arc;

pub(crate) const COMMAND_NAMES: &[&str] = &[
    "get_daemon_status",
    "retry_daemon",
    "quit_desktop",
    "restart_desktop",
    "validate_daemon_connection",
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
            get_daemon_status,
            retry_daemon,
            quit_desktop,
            restart_desktop,
            validate_daemon_connection,
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
fn get_daemon_status(supervisor: tauri::State<'_, Arc<DaemonSupervisionUsecase>>) -> DaemonStatus {
    supervisor.status()
}
#[tauri::command]
fn retry_daemon(supervisor: tauri::State<'_, Arc<DaemonSupervisionUsecase>>) -> Result<(), String> {
    supervisor.retry().map_err(Into::into)
}
#[tauri::command]
fn quit_desktop(supervisor: tauri::State<'_, Arc<DaemonSupervisionUsecase>>) -> Result<(), String> {
    supervisor.stop(StopIntent::Quit(0)).map_err(Into::into)
}
#[tauri::command]
fn restart_desktop(
    supervisor: tauri::State<'_, Arc<DaemonSupervisionUsecase>>,
) -> Result<(), String> {
    supervisor.stop(StopIntent::Restart).map_err(Into::into)
}
#[tauri::command]
fn validate_daemon_connection(
    supervisor: tauri::State<'_, Arc<DaemonSupervisionUsecase>>,
    launch_id: String,
    release: String,
) -> Result<(), String> {
    supervisor
        .validate_connection(&launch_id, &release)
        .map_err(Into::into)
}
#[tauri::command]
async fn get_login_item_status(
    login: tauri::State<'_, crate::usecase::login_item::LoginItemUsecase>,
) -> Result<crate::usecase::login_item::LoginItemState, String> {
    login.status().await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn set_login_item_enabled(
    login: tauri::State<'_, crate::usecase::login_item::LoginItemUsecase>,
    enabled: bool,
) -> Result<crate::usecase::login_item::LoginItemState, String> {
    login.set_enabled(enabled).await.map_err(|e| e.to_string())
}
#[tauri::command]
fn open_login_item_settings(
    login: tauri::State<'_, crate::usecase::login_item::LoginItemUsecase>,
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
