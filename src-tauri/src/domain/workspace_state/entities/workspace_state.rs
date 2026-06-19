use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};

#[derive(Clone, Debug)]
pub struct WorkspaceState {
    pub version: u32,
    pub tabs: WorkspaceTabsState,
    pub layout: WorkspaceLayoutState,
}
