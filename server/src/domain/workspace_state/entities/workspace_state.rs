use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceState {
    pub panes: Option<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>,
    pub version: u32,
    pub tabs: WorkspaceTabsState,
    pub layout: WorkspaceLayoutState,
}

impl WorkspaceState {
    pub fn preserve_omitted_panes(mut self, previous: Option<Self>) -> Self {
        if self.panes.is_none() {
            self.panes = previous.and_then(|state| state.panes);
        }
        self
    }
}

#[cfg(test)]
#[path = "workspace_state_test.rs"]
mod workspace_state_tests;
