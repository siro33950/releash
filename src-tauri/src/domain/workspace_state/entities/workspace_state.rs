use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceState {
    pub version: u32,
    pub tabs: WorkspaceTabsState,
    pub layout: WorkspaceLayoutState,
}
