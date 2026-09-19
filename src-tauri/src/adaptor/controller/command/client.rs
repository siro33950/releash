use super::CommandRouter;

pub(crate) const COMMAND_NAMES: &[&str] = &[
    "complete_desktop_restoration",
    "fail_desktop_restoration",
    "apply_desktop_settings",
    "get_client_endpoint",
];

pub(crate) fn register<R: tauri::Runtime>(router: &mut CommandRouter<super::InvokeHandler<R>>) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![
            complete_desktop_restoration,
            fail_desktop_restoration,
            apply_desktop_settings,
            get_client_endpoint
        ]),
    );
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientEndpoint {
    #[serde(flatten)]
    endpoint: crate::usecase::client_connection::ClientConnectionDto,
    launch_id: String,
}

#[tauri::command]
async fn get_client_endpoint(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    attachment_id: String,
) -> Result<ClientEndpoint, String> {
    let connection = supervisor
        .attach(attachment_id)
        .await
        .map_err(String::from)?;
    Ok(ClientEndpoint {
        endpoint: connection.endpoint,
        launch_id: connection.launch_id,
    })
}

#[tauri::command]
async fn complete_desktop_restoration(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    launch_id: String,
    attachment_id: String,
    generation: u64,
) -> Result<(), String> {
    supervisor
        .finish_restoration(&launch_id, &attachment_id, generation)
        .await
        .map_err(Into::into)
}

#[tauri::command]
fn fail_desktop_restoration(
    supervisor: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>,
    >,
    generation: u64,
    reason: String,
) {
    supervisor.fail_restoration(generation, reason);
}
