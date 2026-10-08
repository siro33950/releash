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
) -> Result<crate::adaptor::presenter::daemon_connection::DesktopEndpoint, String> {
    let endpoint = super::super::desktop_lifecycle::connection(
        &app,
        |lifecycle, failure_window| Box::pin(lifecycle.endpoint(failure_window)),
        |_| {},
    )
    .await
    .map_err(crate::adaptor::presenter::daemon_connection::message)?;
    Ok(crate::adaptor::presenter::daemon_connection::endpoint(
        endpoint,
    ))
}
