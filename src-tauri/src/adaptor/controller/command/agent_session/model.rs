use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, SessionHandle, CODEX_BACKEND_ID,
};
use crate::usecase::agent_session::session::SessionStore;

#[tauri::command]
pub async fn set_agent_permission_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    handles: tauri::State<'_, Arc<Mutex<AgentProcessMap>>>,
    registry: tauri::State<'_, Arc<AgentBackendRegistry>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    let pm = crate::permission::PermissionMode::parse(&permission_mode)
        .map_err(|error| error.to_string())?;
    let data_dir = crate::app_data_dir::resolve_data_dir(&app)?;
    let meta = session_store
        .get_session_meta(&data_dir, &chat_session_id)?
        .ok_or_else(|| format!("Session not found: {chat_session_id}"))?;

    if meta.backend_id.as_deref() == Some(CODEX_BACKEND_ID) {
        session_store.update_permission_mode(&data_dir, &chat_session_id, pm.as_str())?;
        session_store.update_permission_profile_id(&data_dir, &chat_session_id, None)?;
        if let Some(backend) = registry.get(CODEX_BACKEND_ID) {
            if let Err(error) = backend
                .set_permission_mode(
                    &SessionHandle {
                        chat_session_id: chat_session_id.clone(),
                        backend_id: CODEX_BACKEND_ID.to_string(),
                    },
                    &meta.worktree_path,
                    pm.as_str(),
                )
                .await
            {
                log::debug!(
                    "skipped Codex runtime permission mode sync for {chat_session_id}: {error}"
                );
            }
        }
        return Ok(());
    }

    crate::infrastructure::agent_session::runtime::set_agent_permission_mode(
        app,
        session_store,
        handles,
        chat_session_id,
        pm.as_str().to_string(),
    )
    .await
}

#[tauri::command]
pub async fn set_agent_plan_mode(
    app: tauri::AppHandle,
    session_store: tauri::State<'_, Arc<SessionStore>>,
    chat_session_id: String,
    plan_mode: bool,
) -> Result<(), String> {
    let data_dir = crate::app_data_dir::resolve_data_dir(&app)?;
    session_store.update_plan_mode(&data_dir, &chat_session_id, plan_mode)
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
