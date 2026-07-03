use tauri::State;

use crate::adaptor::controller_support::AgentSessionRuntimeState;
use crate::domain::agent_session::SkillEntry;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntryMsg {
    pub name: String,
    pub description: String,
    pub scope: String,
}

impl From<SkillEntry> for SkillEntryMsg {
    fn from(value: SkillEntry) -> Self {
        Self {
            name: value.name,
            description: value.description,
            scope: value.scope,
        }
    }
}

#[tauri::command]
pub async fn scan_agent_skills(
    runtime: State<'_, AgentSessionRuntimeState>,
    cwd: String,
    backend_id: Option<String>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SkillEntryMsg>, String> {
    runtime
        .skill_catalog(
            backend_id.as_deref(),
            std::path::Path::new(&cwd),
            query.as_deref(),
            limit,
        )
        .await
        .map(|skills| skills.into_iter().map(SkillEntryMsg::from).collect())
        .map_err(|error| error.to_string())
}
