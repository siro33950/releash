use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::SessionStore;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn respond_agent_permission(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    request_id: String,
    behavior: String,
    message: Option<String>,
    updated_input: Option<String>,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::respond_agent_permission(
        app,
        session_store,
        handles,
        chat_session_id,
        request_id,
        behavior,
        message,
        updated_input,
    )
    .await
}
