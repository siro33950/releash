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

use crate::domain::workspace_state::value_objects::pane_layout::{PaneTab, PaneTabKind};
use crate::domain::workspace_state::value_objects::{
    workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct StoredWorkspaceState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panes: Option<StoredPaneLayout>,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tabs: Option<StoredWorkspaceTabsState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<StoredWorkspaceLayoutState>,
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
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorkspaceLayoutState {
    center_tab: String,
    active_view: String,
    left_nav_collapsed: bool,
    right_collapsed: bool,
    right_bottom_collapsed: bool,
}

impl From<&WorkspaceState> for StoredWorkspaceState {
    fn from(state: &WorkspaceState) -> Self {
        Self {
            panes: state.panes.as_ref().map(Into::into),
            version: state.version,
            tabs: state.tabs.clone().map(Into::into),
            layout: state.layout.clone().map(Into::into),
        }
    }
}

impl TryFrom<StoredWorkspaceState> for WorkspaceState {
    type Error = WorkspaceStateError;
    fn try_from(dto: StoredWorkspaceState) -> Result<Self, Self::Error> {
        Ok(Self {
            panes: dto.panes.map(TryInto::try_into).transpose()?,
            version: dto.version,
            tabs: dto.tabs.map(Into::into),
            layout: dto.layout.map(Into::into),
        })
    }
}

impl From<WorkspaceTabsState> for StoredWorkspaceTabsState {
    fn from(tabs: WorkspaceTabsState) -> Self {
        Self {
            editors: tabs.editors.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<StoredWorkspaceTabsState> for WorkspaceTabsState {
    fn from(dto: StoredWorkspaceTabsState) -> Self {
        Self {
            editors: dto.editors.into_iter().map(Into::into).collect(),
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
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPaneTabKind {
    Terminal,
    File,
    Session,
    Command,
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
    /// 保存済みの配置を domain の配置に変える。
    ///
    /// 以前の版が保存したファイルタブ（`kind` が `file` のタブ）は、配置に入れずに落とす。
    /// 落としたタブが選択中だったときは、その pane の選択を、落とさなかったタブの中から決める。
    fn try_from(value: StoredPaneLayout) -> Result<Self, Self::Error> {
        match value {
            StoredPaneLayout::Pane {
                id,
                tabs,
                mut active_tab,
            } => {
                let removed_active = tabs.iter().any(|tab| {
                    tab.kind == StoredPaneTabKind::File && active_tab.as_ref() == Some(&tab.id)
                });
                let tabs: Vec<_> = tabs
                    .into_iter()
                    .filter_map(|tab| {
                        let kind = match tab.kind {
                            StoredPaneTabKind::File => return None,
                            StoredPaneTabKind::Terminal => PaneTabKind::Terminal,
                            StoredPaneTabKind::Session => PaneTabKind::Session,
                            StoredPaneTabKind::Command => PaneTabKind::Command,
                        };
                        Some(PaneTab {
                            id: tab.id,
                            kind,
                            node_id: tab.node_id,
                            agent_session_id: tab.agent_session_id,
                        })
                    })
                    .collect();
                if removed_active {
                    active_tab = tabs.first().map(|tab| tab.id.clone());
                }
                Self::pane(id, tabs, active_tab)
            }
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
impl From<PaneTab> for StoredPaneTab {
    fn from(value: PaneTab) -> Self {
        Self {
            id: value.id,
            kind: value.kind.into(),
            node_id: value.node_id,
            agent_session_id: value.agent_session_id,
        }
    }
}
impl From<PaneTabKind> for StoredPaneTabKind {
    fn from(value: PaneTabKind) -> Self {
        match value {
            PaneTabKind::Terminal => Self::Terminal,
            PaneTabKind::Command => Self::Command,
            PaneTabKind::Session => Self::Session,
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
