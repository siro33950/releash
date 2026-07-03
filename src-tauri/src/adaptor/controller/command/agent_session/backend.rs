use std::sync::Arc;

use tauri::State;

use crate::usecase::agent_session::backend_registry::BackendListResult;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;

#[tauri::command]
pub fn list_agent_backends(
    runtime: State<'_, Arc<AgentSessionRuntimeUsecase>>,
) -> BackendListResult {
    runtime.list_backends()
}
