pub(crate) const WORKSPACE_STATE_CHANGED: &str = "workspace.state_changed";
pub(crate) const REPOSITORY_GROUP_CHANGED: &str = "workspace.repository_group_changed";
pub(crate) const WORKSPACE_STATE_INVALIDATED: &str = "workspace.state_invalidated";
fn malformed(event_type: &str) -> EventCodecError {
    EventCodecError::MalformedPayload {
        event_type: event_type.into(),
    }
}
use super::canonical_cbor::CborValue;
use super::envelope::{EventCodecError, LocalEventPayloadCodec};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateError};

pub(crate) struct WorkspaceStateCodec;
impl LocalEventPayloadCodec for WorkspaceStateCodec {
    fn event_type(&self) -> &'static str {
        WORKSPACE_STATE_CHANGED
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::WorkspaceStateChanged(_))
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::WorkspaceStateChanged(value) = event else {
            return Err(malformed(self.event_type()));
        };
        serde_json::to_string(&StoredWorkspaceState::from(value.as_ref()))
            .map(CborValue::Text)
            .map_err(|_| malformed(self.event_type()))
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed(self.event_type()));
        };
        let stored: StoredWorkspaceState =
            serde_json::from_str(raw).map_err(|_| malformed(self.event_type()))?;
        let state = stored
            .try_into()
            .map_err(|_| malformed(self.event_type()))?;
        Ok(Some(LocalDomainEvent::WorkspaceStateChanged(Box::new(
            state,
        ))))
    }
}

pub(crate) struct RepositoryGroupCodec;
impl LocalEventPayloadCodec for RepositoryGroupCodec {
    fn event_type(&self) -> &'static str {
        REPOSITORY_GROUP_CHANGED
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::RepositoryGroupChanged(_))
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        let LocalDomainEvent::RepositoryGroupChanged(value) = event else {
            return Err(malformed(self.event_type()));
        };
        serde_json::to_string(value)
            .map(CborValue::Text)
            .map_err(|_| malformed(self.event_type()))
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        let CborValue::Text(raw) = value else {
            return Err(malformed(self.event_type()));
        };
        serde_json::from_str(raw)
            .map(LocalDomainEvent::RepositoryGroupChanged)
            .map(Some)
            .map_err(|_| malformed(self.event_type()))
    }
}

pub(crate) struct WorkspaceStateInvalidationCodec;

impl LocalEventPayloadCodec for WorkspaceStateInvalidationCodec {
    fn event_type(&self) -> &'static str {
        WORKSPACE_STATE_INVALIDATED
    }
    fn payload_version(&self) -> i64 {
        1
    }
    fn handles(&self, event: &LocalDomainEvent) -> bool {
        matches!(event, LocalDomainEvent::WorkspaceStateInvalidated)
    }
    fn encode(&self, event: &LocalDomainEvent) -> Result<CborValue, EventCodecError> {
        if self.handles(event) {
            Ok(CborValue::Null)
        } else {
            Err(malformed(self.event_type()))
        }
    }
    fn decode(
        &self,
        version: i64,
        value: &CborValue,
    ) -> Result<Option<LocalDomainEvent>, EventCodecError> {
        if version != 1 {
            return Ok(None);
        }
        if *value == CborValue::Null {
            Ok(Some(LocalDomainEvent::WorkspaceStateInvalidated))
        } else {
            Err(malformed(self.event_type()))
        }
    }
}

#[cfg(test)]
#[path = "workspace_state_codec_test.rs"]
mod workspace_state_codec_tests;

use crate::domain::workspace_state::value_objects::{
    workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct StoredWorkspaceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panes: Option<StoredPaneLayout>,
    version: u32,
    tabs: StoredWorkspaceTabsState,
    layout: StoredWorkspaceLayoutState,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredWorkspaceTabEntry {
    path: String,
    name: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorkspaceTabsState {
    editors: Vec<StoredWorkspaceTabEntry>,
    active_editor_path: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorkspaceLayoutState {
    center_tab: String,
    active_view: String,
    left_nav_collapsed: bool,
    right_collapsed: bool,
    right_bottom_collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    right_bottom_active_tab: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_diff_file: Option<String>,
}

impl From<&WorkspaceState> for StoredWorkspaceState {
    fn from(state: &WorkspaceState) -> Self {
        Self {
            panes: state.panes.as_ref().map(Into::into),
            version: state.version,
            tabs: state.tabs.clone().into(),
            layout: state.layout.clone().into(),
        }
    }
}

impl TryFrom<StoredWorkspaceState> for WorkspaceState {
    type Error = WorkspaceStateError;
    fn try_from(dto: StoredWorkspaceState) -> Result<Self, Self::Error> {
        Ok(Self {
            panes: dto.panes.map(TryInto::try_into).transpose()?,
            version: dto.version,
            tabs: dto.tabs.into(),
            layout: dto.layout.into(),
        })
    }
}

impl From<WorkspaceTabsState> for StoredWorkspaceTabsState {
    fn from(tabs: WorkspaceTabsState) -> Self {
        Self {
            editors: tabs.editors.into_iter().map(Into::into).collect(),
            active_editor_path: tabs.active_editor_path,
        }
    }
}

impl From<StoredWorkspaceTabsState> for WorkspaceTabsState {
    fn from(dto: StoredWorkspaceTabsState) -> Self {
        Self {
            editors: dto.editors.into_iter().map(Into::into).collect(),
            active_editor_path: dto.active_editor_path,
        }
    }
}

impl From<WorkspaceTabEntry> for StoredWorkspaceTabEntry {
    fn from(entry: WorkspaceTabEntry) -> Self {
        Self {
            path: entry.path,
            name: entry.name,
        }
    }
}

impl From<StoredWorkspaceTabEntry> for WorkspaceTabEntry {
    fn from(dto: StoredWorkspaceTabEntry) -> Self {
        Self {
            path: dto.path,
            name: dto.name,
        }
    }
}

impl From<WorkspaceLayoutState> for StoredWorkspaceLayoutState {
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

impl From<StoredWorkspaceLayoutState> for WorkspaceLayoutState {
    fn from(dto: StoredWorkspaceLayoutState) -> Self {
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
enum StoredPaneLayout {
    Pane {
        id: String,
        tabs: Vec<StoredPaneTab>,
        active_tab: Option<String>,
    },
    Split {
        id: String,
        axis: StoredSplitAxis,
        ratio: f64,
        first: Box<StoredPaneLayout>,
        second: Box<StoredPaneLayout>,
    },
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct StoredPaneTab {
    pub id: String,
    pub kind: StoredPaneTabKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_base: Option<String>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPaneTabKind {
    Terminal,
    Workflow,
    File,
    Session,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSplitAxis {
    Horizontal,
    Vertical,
}

impl From<&crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>
    for StoredPaneLayout
{
    fn from(
        value: &crate::domain::workspace_state::value_objects::pane_layout::PaneLayout,
    ) -> Self {
        use crate::domain::workspace_state::value_objects::pane_layout::PaneLayoutView;
        match value.view() {
            PaneLayoutView::Pane {
                id,
                tabs,
                active_tab,
            } => Self::Pane {
                id: id.into(),
                tabs: tabs.iter().cloned().map(Into::into).collect(),
                active_tab: active_tab.map(str::to_owned),
            },
            PaneLayoutView::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                id: id.into(),
                axis: axis.clone().into(),
                ratio,
                first: Box::new(first.into()),
                second: Box::new(second.into()),
            },
        }
    }
}
impl TryFrom<StoredPaneLayout>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneLayout
{
    type Error = WorkspaceStateError;
    fn try_from(value: StoredPaneLayout) -> Result<Self, Self::Error> {
        match value {
            StoredPaneLayout::Pane {
                id,
                tabs,
                active_tab,
            } => Self::pane(
                id,
                tabs.into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
                active_tab,
            ),
            StoredPaneLayout::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => Self::split(
                id,
                axis.into(),
                ratio,
                (*first).try_into()?,
                (*second).try_into()?,
            ),
        }
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneTab> for StoredPaneTab {
    fn from(value: crate::domain::workspace_state::value_objects::pane_layout::PaneTab) -> Self {
        Self {
            id: value.id,
            kind: value.kind.into(),
            file_path: value.file_path,
            file_mode: value.file_mode.map(|mode| mode.as_str().into()),
            file_base: value.file_base.map(|base| base.as_str().into()),
        }
    }
}
impl TryFrom<StoredPaneTab>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneTab
{
    type Error = WorkspaceStateError;
    fn try_from(value: StoredPaneTab) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            kind: value.kind.into(),
            file_path: value.file_path,
            file_mode: value
                .file_mode
                .as_deref()
                .map(crate::domain::code::value_objects::review::FileMode::parse)
                .transpose()
                .map_err(|_| WorkspaceStateError::InvalidLayout)?,
            file_base: value
                .file_base
                .as_deref()
                .map(crate::domain::code::ReviewBase::parse)
                .transpose()
                .map_err(|_| WorkspaceStateError::InvalidLayout)?,
        })
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind>
    for StoredPaneTabKind
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
            crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind::Session => {
                Self::Session
            }
        }
    }
}
impl From<StoredPaneTabKind>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneTabKind
{
    fn from(value: StoredPaneTabKind) -> Self {
        match value {
            StoredPaneTabKind::Terminal => Self::Terminal,
            StoredPaneTabKind::Workflow => Self::Workflow,
            StoredPaneTabKind::File => Self::File,
            StoredPaneTabKind::Session => Self::Session,
        }
    }
}
impl From<crate::domain::workspace_state::value_objects::pane_layout::SplitAxis>
    for StoredSplitAxis
{
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
impl From<StoredSplitAxis>
    for crate::domain::workspace_state::value_objects::pane_layout::SplitAxis
{
    fn from(value: StoredSplitAxis) -> Self {
        match value {
            StoredSplitAxis::Horizontal => Self::Horizontal,
            StoredSplitAxis::Vertical => Self::Vertical,
        }
    }
}
