#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceTabEntry {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceTabsState {
    pub editors: Vec<WorkspaceTabEntry>,
    pub active_editor_path: Option<String>,
}
