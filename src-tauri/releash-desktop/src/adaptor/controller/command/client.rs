use super::CommandRouter;

pub(crate) const COMMAND_NAMES: &[&str] = &["get_client_endpoint"];

pub(crate) fn register<R: tauri::Runtime>(router: &mut CommandRouter<super::InvokeHandler<R>>) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(tauri::generate_handler![get_client_endpoint]),
    );
}

#[cfg(test)]
#[path = "client_test.rs"]
pub(crate) mod client_tests;

#[tauri::command]
pub(crate) async fn get_client_endpoint<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    connection: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::daemon_connection::DaemonConnectionUsecase>,
    >,
) -> Result<releashd::desktop_api::ClientConnectionDto, String> {
    let (endpoint, changed) = connection.endpoint().await.map_err(|e| e.to_string())?;
    if changed {
        super::super::desktop_lifecycle::connected(&app, false);
    }
    Ok(releashd::desktop_api::ClientConnectionDto {
        url: endpoint.url,
        token: endpoint.token,
    })
}
