use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use super::facts;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::workspace_state::services::filter_missing_files;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};

pub struct WorkspaceStateStore {
    app_data_dir: PathBuf,
    store: OnceLock<Arc<LocalEventStore>>,
    entries: RwLock<HashMap<String, WorkspaceState>>,
    file_lock: parking_lot::Mutex<()>,
}

impl WorkspaceStateStore {
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            store: OnceLock::new(),
            entries: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
        }
    }

    pub fn with_store(app_data_dir: PathBuf, store: Arc<LocalEventStore>) -> Self {
        Self {
            app_data_dir,
            store: OnceLock::from(store),
            entries: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
        }
    }

    fn event_store(&self) -> Result<&Arc<LocalEventStore>, WorkspaceStateError> {
        if let Some(store) = self.store.get() {
            return Ok(store);
        }
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            self.app_data_dir.clone(),
            Arc::new(crate::common::retry::RetryLimiter::default()),
        ))
        .map_err(facts::error)?;
        self.store
            .set(store)
            .map_err(|_| facts::error("UI state store was initialized concurrently"))?;
        Ok(self.store.get().expect("initialized event store"))
    }

    fn read_state(&self, key: &str) -> Result<Option<WorkspaceState>, WorkspaceStateError> {
        let (_, event) = facts::run(facts::latest(
            self.event_store()?,
            &facts::stream(key, false),
        ))?;
        match event {
            Some(LocalDomainEvent::WorkspaceStateChanged(state)) => Ok(Some(*state)),
            None | Some(LocalDomainEvent::WorkspaceStateInvalidated) => Ok(None),
            _ => Err(facts::error("Unexpected workspace state event")),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn get(&self, key: &str) -> Option<WorkspaceState> {
        self.entries.read().get(key).cloned()
    }
}

#[cfg(any(test, feature = "test-support"))]
fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("workspace_state")
}
pub(crate) const ABSOLUTE_STATE_KEY_PREFIX: &str = "worktree-";
pub fn storage_key(key: &str) -> String {
    if Path::new(key).is_absolute() {
        use sha2::Digest;
        format!(
            "{ABSOLUTE_STATE_KEY_PREFIX}{}",
            hex::encode(sha2::Sha256::digest(key.as_bytes()))
        )
    } else {
        key.replace(['/', '\\'], "_")
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn state_file(app_data_dir: &Path, key: &str) -> PathBuf {
    state_dir(app_data_dir).join(format!("{}.json", storage_key(key)))
}
pub(super) fn repository_group_key(path: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(
        crate::domain::repository::normalize_repo_path(path).as_bytes(),
    ))
}

impl WorkspaceStateRepository for WorkspaceStateStore {
    fn migrate_legacy(
        &self,
        repositories: &[String],
        worktrees: &[String],
    ) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        super::legacy_import::migrate(
            &self.app_data_dir,
            self.event_store()?,
            repositories,
            worktrees,
        )
    }
    fn load_repository_group(&self, path: &str) -> Result<bool, WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        let (_, event) = facts::run(facts::latest(
            self.event_store()?,
            &facts::stream(path, true),
        ))?;
        match event {
            Some(LocalDomainEvent::RepositoryGroupChanged(collapsed)) => Ok(collapsed),
            None => Ok(false),
            _ => Err(facts::error("Unexpected repository group event")),
        }
    }
    fn save_repository_group(
        &self,
        path: &str,
        collapsed: bool,
    ) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        facts::run(facts::append(
            self.event_store()?,
            facts::stream(path, true),
            LocalDomainEvent::RepositoryGroupChanged(collapsed),
        ))
    }
    fn load(&self, key: &str, root: &str) -> Result<Option<WorkspaceState>, WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        let Some(state) = self.read_state(key)? else {
            return Ok(None);
        };
        Ok(Some(filter_missing_files(state, root)))
    }
    fn check_readable(&self, key: &str) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        self.read_state(key).map(|_| ())
    }
    fn save(&self, key: &str) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        let mut entries = self.entries.write();
        let Some(state) = entries.get(key).cloned() else {
            return Ok(());
        };
        let state = state.preserve_omitted_panes(self.read_state(key)?);
        facts::run(facts::append(
            self.event_store()?,
            facts::stream(key, false),
            LocalDomainEvent::WorkspaceStateChanged(Box::new(state)),
        ))?;
        entries.remove(key);
        Ok(())
    }
    fn set(&self, key: &str, state: WorkspaceState) {
        self.entries.write().insert(key.into(), state);
    }
}

impl crate::usecase::app_data_gc::WorkspaceStateGc for WorkspaceStateStore {
    fn states(
        &self,
    ) -> Result<Vec<(String, crate::domain::local_event::StreamVersion)>, WorkspaceStateError> {
        facts::run(async {
            self.event_store()?.submit_query(|connection| {
                let mut statement = connection.prepare("SELECT stream_id, stream_sequence FROM events e WHERE stream_id LIKE 'workspace-state:%' AND event_type = 'workspace.state_changed' AND NOT EXISTS (SELECT 1 FROM events next WHERE next.stream_id = e.stream_id AND next.stream_sequence > e.stream_sequence)").map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))?;
                let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))?;
                rows.collect::<Result<Vec<_>, _>>().map_err(|error| crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error))
            }).await.map_err(facts::error)?.into_iter().map(|(stream, version)| {
                let hash = stream.strip_prefix("workspace-state:").ok_or_else(|| facts::error("Unexpected workspace state stream"))?;
                Ok((format!("{ABSOLUTE_STATE_KEY_PREFIX}{hash}"), crate::domain::local_event::StreamVersion::new(version).map_err(facts::error)?))
            }).collect()
        })
    }
    fn invalidate(
        &self,
        key: &str,
        expected: crate::domain::local_event::StreamVersion,
    ) -> Result<bool, WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        let hash = key
            .strip_prefix(ABSOLUTE_STATE_KEY_PREFIX)
            .ok_or_else(|| facts::error("Unexpected workspace state key"))?;
        let stream =
            crate::domain::local_event::StreamId::parse(&format!("workspace-state:{hash}"))
                .map_err(facts::error)?;
        let (head, event) = facts::run(facts::latest(self.event_store()?, &stream))?;
        if head != expected || !matches!(event, Some(LocalDomainEvent::WorkspaceStateChanged(_))) {
            return Ok(false);
        }
        facts::run(facts::append(
            self.event_store()?,
            stream,
            LocalDomainEvent::WorkspaceStateInvalidated,
        ))?;
        Ok(true)
    }
}

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
impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>
    for StoredPaneLayout
{
    fn from(value: crate::domain::workspace_state::value_objects::pane_layout::PaneLayout) -> Self {
        Self::from(&value)
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
            } => Self::pane(id, tabs.into_iter().map(Into::into).collect(), active_tab),
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
        }
    }
}
impl From<StoredPaneTab> for crate::domain::workspace_state::value_objects::pane_layout::PaneTab {
    fn from(value: StoredPaneTab) -> Self {
        Self {
            id: value.id,
            kind: value.kind.into(),
        }
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

#[cfg(test)]
#[path = "repository_impl_test.rs"]
mod repository_impl_tests;
