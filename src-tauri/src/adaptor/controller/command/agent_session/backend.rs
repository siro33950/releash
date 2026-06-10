use std::sync::Arc;

use tauri::State;

use crate::infrastructure::agent_session::runtime::{AgentBackendRegistry, BackendListResult};

#[tauri::command]
pub fn list_agent_backends(registry: State<'_, Arc<AgentBackendRegistry>>) -> BackendListResult {
    crate::infrastructure::agent_session::runtime::list_agent_backends(registry)
}
