use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};
use crate::domain::workspace_state::WorkspaceState;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceStateDto {
    pub version: u32,
    pub tabs: WorkspaceTabsState,
    pub layout: WorkspaceLayoutStateDto,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLayoutStateDto {
    pub center_tab: String,
    pub active_view: String,
    pub left_nav_collapsed: bool,
    pub right_collapsed: bool,
    pub right_bottom_collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_bottom_active_tab: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_diff_file: Option<String>,
}

impl From<WorkspaceState> for WorkspaceStateDto {
    fn from(state: WorkspaceState) -> Self {
        Self {
            version: state.version,
            tabs: state.tabs,
            layout: state.layout.into(),
        }
    }
}

impl From<WorkspaceStateDto> for WorkspaceState {
    fn from(dto: WorkspaceStateDto) -> Self {
        Self {
            version: dto.version,
            tabs: dto.tabs,
            layout: dto.layout.into(),
        }
    }
}

impl From<WorkspaceLayoutState> for WorkspaceLayoutStateDto {
    fn from(layout: WorkspaceLayoutState) -> Self {
        Self {
            center_tab: layout.center_tab,
            active_view: layout.active_view,
            left_nav_collapsed: layout.left_nav_collapsed,
            right_collapsed: layout.right_collapsed,
            right_bottom_collapsed: layout.right_bottom_collapsed,
            right_bottom_active_tab: layout.right_bottom_active_tab,
            selected_diff_file: layout.selected_diff_file,
        }
    }
}

impl From<WorkspaceLayoutStateDto> for WorkspaceLayoutState {
    fn from(dto: WorkspaceLayoutStateDto) -> Self {
        Self {
            center_tab: dto.center_tab,
            active_view: dto.active_view,
            left_nav_collapsed: dto.left_nav_collapsed,
            right_collapsed: dto.right_collapsed,
            right_bottom_collapsed: dto.right_bottom_collapsed,
            right_bottom_active_tab: dto.right_bottom_active_tab,
            selected_diff_file: dto.selected_diff_file,
        }
    }
}
