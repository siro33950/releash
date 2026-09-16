use super::CommandRouter;
use crate::adaptor::protocol::client::ClientEndpoint;
use tauri::Manager;

pub(crate) const COMMAND_NAMES: &[&str] = &["get_client_endpoint", "apply_desktop_settings"];

pub(crate) fn register<R: tauri::Runtime>(router: &mut CommandRouter<super::InvokeHandler<R>>) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![
            get_client_endpoint,
            apply_desktop_settings
        ]),
    );
}

#[tauri::command(async)]
fn get_client_endpoint<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<ClientEndpoint, String> {
    client_endpoint(&app)
}

pub(crate) fn client_endpoint<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<ClientEndpoint, String> {
    let connection = app
        .try_state::<crate::usecase::client_connection::ClientConnectionUsecase>()
        .ok_or("desktop client connection is unavailable")?;
    connection
        .endpoint()
        .map(|endpoint| ClientEndpoint {
            url: endpoint.url,
            auth_subprotocol: endpoint.auth_subprotocol,
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "client_test.rs"]
pub(crate) mod client_tests;

#[tauri::command]
fn apply_desktop_settings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    settings: crate::usecase::app_config::query_service::DesktopSettingsDto,
) {
    crate::desktop::apply_desktop_settings(&app, settings);
}
