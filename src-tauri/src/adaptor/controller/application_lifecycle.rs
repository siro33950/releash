use std::sync::Arc;

pub(crate) fn request_application_quit_with_runtime(
    app: tauri::AppHandle,
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
) {
    tauri::async_runtime::spawn(async move {
        // Kill all agent sessions before stopping the server.
        runtime.close_all().await;
        app.exit(0);
    });
}
