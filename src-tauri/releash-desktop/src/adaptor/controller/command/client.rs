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
pub(crate) async fn get_client_endpoint(
    connection: tauri::State<
        '_,
        std::sync::Arc<crate::usecase::desktop_lifecycle::DesktopLifecycleUsecase>,
    >,
) -> Result<releashd::desktop_api::ClientConnectionDto, String> {
    let endpoint = connection
        .endpoint()
        .await
        .map_err(crate::adaptor::presenter::daemon_connection::message)?;
    Ok(releashd::desktop_api::ClientConnectionDto {
        url: endpoint.url,
        token: endpoint.token,
    })
}
