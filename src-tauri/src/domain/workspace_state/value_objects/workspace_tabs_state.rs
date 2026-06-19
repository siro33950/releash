#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceTabEntry {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTabsState {
    pub editors: Vec<WorkspaceTabEntry>,
    pub active_editor_path: Option<String>,
}
