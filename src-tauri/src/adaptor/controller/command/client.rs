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
) -> Result<ClientEndpoint, String> {
    let connection = supervisor.attach().await.map_err(String::from)?;
    Ok(ClientEndpoint {
        endpoint: connection.endpoint,
        launch_id: connection.launch_id,
    })
}
