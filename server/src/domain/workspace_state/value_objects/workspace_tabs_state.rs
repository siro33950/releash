#[derive(Clone, Debug)]
pub struct WorkspaceTabEntry {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct WorkspaceTabsState {
    pub editors: Vec<WorkspaceTabEntry>,
    pub active_editor_path: Option<String>,
}
