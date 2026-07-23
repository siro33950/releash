use std::sync::Arc;

use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;

#[tauri::command]
pub async fn set_agent_permission_mode(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    runtime: tauri::State<'_, Arc<AgentSessionRuntimeUsecase>>,
    chat_session_id: String,
    permission_mode: String,
) -> Result<(), String> {
    super::session::ensure_mutation_admission_message(store.inner().as_ref()).await?;
    let mode = crate::domain::agent_session::PermissionMode::parse(&permission_mode)
        .map_err(|error| error.to_string())?;
    runtime
        .set_permission_mode(&chat_session_id, mode)
        .await
        .map_err(|error| super::session::normalize_mutation_error(error.to_string()))
}

#[tauri::command]
pub async fn set_agent_plan_mode(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    runtime: tauri::State<'_, Arc<AgentSessionRuntimeUsecase>>,
    chat_session_id: String,
    plan_mode: bool,
) -> Result<(), String> {
    super::session::ensure_mutation_admission_message(store.inner().as_ref()).await?;
    runtime
        .set_plan_mode(&chat_session_id, plan_mode)
        .await
        .map_err(|error| super::session::normalize_mutation_error(error.to_string()))
}

#[tauri::command]
pub async fn set_agent_model(
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    runtime: tauri::State<'_, Arc<AgentSessionRuntimeUsecase>>,
    chat_session_id: String,
    model_id: String,
) -> Result<(), String> {
    super::session::ensure_mutation_admission_message(store.inner().as_ref()).await?;
    runtime
        .set_model(&chat_session_id, &model_id)
        .await
        .map_err(|error| super::session::normalize_mutation_error(error.to_string()))
}
