use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::AgentProcessMap;

pub(crate) fn request_application_quit(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Kill all agent sessions before stopping the server.
        if let Some(handles) = app.try_state::<Arc<Mutex<AgentProcessMap>>>() {
            crate::infrastructure::agent_session::runtime::close_all_agent_sessions(
                &app,
                handles.inner(),
            )
            .await;
        }
        app.exit(0);
    });
}
