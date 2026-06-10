use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, AgentProcessMap};
use crate::usecase::agent_session::session::SessionStore;

#[tauri::command]
pub async fn set_agent_permission_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::set_agent_permission_mode(
        app,
        session_store,
        handles,
        chat_session_id,
        permission_mode,
    )
    .await
}

#[tauri::command]
pub async fn set_agent_model(
    app: tauri::AppHandle,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    model_id: String,
) -> Result<(), String> {
    crate::infrastructure::agent_session::runtime::set_agent_model(
        app,
        handles,
        session_store,
        registry,
        chat_session_id,
        model_id,
    )
    .await
}
