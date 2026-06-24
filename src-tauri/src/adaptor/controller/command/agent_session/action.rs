use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::domain::agent_session::SkillEntry;

#[tauri::command]
pub async fn scan_agent_skills(
    state: State<'_, AppState>,
    cwd: String,
    backend_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    state
        .agent_session_usecase
        .scan_agent_skills(cwd, backend_id, query, limit)
        .await
}

#[tauri::command]
pub async fn read_codex_skill_catalog(
    state: State<'_, AppState>,
    cwd: String,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntry>, String> {
    state
        .agent_session_usecase
        .read_codex_skill_catalog(cwd, query, limit)
        .await
}
