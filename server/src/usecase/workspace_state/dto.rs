use crate::domain::workspace_state::value_objects::{
    workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
};
use crate::domain::workspace_state::WorkspaceState;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkspaceStateDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panes: Option<PaneLayoutDto>,
    pub version: u32,
    pub tabs: WorkspaceTabsStateDto,
    pub layout: WorkspaceLayoutStateDto,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WorkspaceTabEntryDto {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTabsStateDto {
    pub editors: Vec<WorkspaceTabEntryDto>,
    pub active_editor_path: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
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
            panes: state.panes.map(Into::into),
            version: state.version,
            tabs: state.tabs.into(),
            layout: state.layout.into(),
        }
    }
}

impl From<WorkspaceStateDto> for WorkspaceState {
    fn from(dto: WorkspaceStateDto) -> Self {
        Self {
            panes: dto.panes.map(Into::into),
            version: dto.version,
            tabs: dto.tabs.into(),
            layout: dto.layout.into(),
        }
    }
}

impl From<WorkspaceTabsState> for WorkspaceTabsStateDto {
    fn from(tabs: WorkspaceTabsState) -> Self {
        Self {
            editors: tabs.editors.into_iter().map(Into::into).collect(),
            active_editor_path: tabs.active_editor_path,
        }
    }
}

impl From<WorkspaceTabsStateDto> for WorkspaceTabsState {
    fn from(dto: WorkspaceTabsStateDto) -> Self {
        Self {
            editors: dto.editors.into_iter().map(Into::into).collect(),
            active_editor_path: dto.active_editor_path,
        }
    }
}

impl From<WorkspaceTabEntry> for WorkspaceTabEntryDto {
    fn from(entry: WorkspaceTabEntry) -> Self {
        Self {
            path: entry.path,
            name: entry.name,
        }
    }
}

impl From<WorkspaceTabEntryDto> for WorkspaceTabEntry {
    fn from(dto: WorkspaceTabEntryDto) -> Self {
        Self {
            path: dto.path,
            name: dto.name,
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

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneLayoutDto {
    Pane {
        id: String,
        tabs: Vec<PaneTabDto>,
        active_tab: Option<String>,
    },
    Split {
        id: String,
        axis: SplitAxisDto,
        ratio: f64,
        first: Box<PaneLayoutDto>,
        second: Box<PaneLayoutDto>,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaneTabDto {
    pub id: String,
    pub kind: PaneTabKindDto,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneTabKindDto {
    Terminal,
    Workflow,
    File,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxisDto {
    Horizontal,
    Vertical,
}

impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>
    for PaneLayoutDto
{
    fn from(value: crate::domain::workspace_state::value_objects::pane_layout::PaneLayout) -> Self {
        use crate::domain::workspace_state::value_objects::pane_layout::PaneLayout;
        match value {
            PaneLayout::Pane {
                id,
                tabs,
                active_tab,
            } => Self::Pane {
                id,
                tabs: tabs.into_iter().map(Into::into).collect(),
                active_tab,
            },
            PaneLayout::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                id,
                axis: axis.into(),
                ratio,
                first: Box::new((*first).into()),
                second: Box::new((*second).into()),
            },
        }
    }
}
impl From<PaneLayoutDto>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneLayout
{
    fn from(value: PaneLayoutDto) -> Self {
        match value {
            PaneLayoutDto::Pane {
                id,
                tabs,
                active_tab,
            } => Self::Pane {
                id,
                tabs: tabs.into_iter().map(Into::into).collect(),
                active_tab,
            },
            PaneLayoutDto::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                id,
                axis: axis.into(),
                ratio,
                first: Box::new((*first).into()),
                second: Box::new((*second).into()),
            },
        }
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneTab> for PaneTabDto {
    fn from(value: crate::domain::workspace_state::value_objects::pane_layout::PaneTab) -> Self {
        Self {
            id: value.id,
            kind: value.kind.into(),
        }
    }
}
impl From<PaneTabDto> for crate::domain::workspace_state::value_objects::pane_layout::PaneTab {
    fn from(value: PaneTabDto) -> Self {
        Self {
            id: value.id,
            kind: value.kind.into(),
        }
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind>
    for PaneTabKindDto
{
    fn from(
        value: crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind,
    ) -> Self {
        match value {
            crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind::Terminal => {
                Self::Terminal
            }
            crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind::Workflow => {
                Self::Workflow
            }
            crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind::File => {
                Self::File
            }
        }
    }
}
impl From<PaneTabKindDto>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind
{
    fn from(value: PaneTabKindDto) -> Self {
        match value {
            PaneTabKindDto::Terminal => Self::Terminal,
            PaneTabKindDto::Workflow => Self::Workflow,
            PaneTabKindDto::File => Self::File,
        }
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::SplitAxis> for SplitAxisDto {
    fn from(value: crate::domain::workspace_state::value_objects::pane_layout::SplitAxis) -> Self {
        match value {
            crate::domain::workspace_state::value_objects::pane_layout::SplitAxis::Horizontal => {
                Self::Horizontal
            }
            crate::domain::workspace_state::value_objects::pane_layout::SplitAxis::Vertical => {
                Self::Vertical
            }
        }
    }
}
impl From<SplitAxisDto> for crate::domain::workspace_state::value_objects::pane_layout::SplitAxis {
    fn from(value: SplitAxisDto) -> Self {
        match value {
            SplitAxisDto::Horizontal => Self::Horizontal,
            SplitAxisDto::Vertical => Self::Vertical,
        }
    }
}

#[cfg(test)]
#[path = "dto_test.rs"]
mod dto_tests;
