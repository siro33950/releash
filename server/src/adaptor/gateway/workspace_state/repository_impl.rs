use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::workspace_state::services::filter_missing_files;
use crate::domain::workspace_state::{
    WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
};
use crate::usecase::workspace_state::dto::WorkspaceStateDto;

pub struct WorkspaceStateStore {
    app_data_dir: PathBuf,
    entries: RwLock<HashMap<String, WorkspaceState>>,
    file_lock: parking_lot::Mutex<()>,
}

impl WorkspaceStateStore {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            app_data_dir,
            entries: RwLock::new(HashMap::new()),
            file_lock: parking_lot::Mutex::new(()),
        }
    }
}

fn state_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("workspace_state")
}

pub(crate) const ABSOLUTE_STATE_KEY_PREFIX: &str = "worktree-";

pub fn storage_key(worktree_name: &str) -> String {
    if std::path::Path::new(worktree_name).is_absolute() {
        use sha2::Digest;
        return format!(
            "{ABSOLUTE_STATE_KEY_PREFIX}{}",
            hex::encode(sha2::Sha256::digest(worktree_name.as_bytes()))
        );
    }
    worktree_name.replace(['/', '\\'], "_")
}

pub fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.json"))
}

impl WorkspaceStateRepository for WorkspaceStateStore {
    fn load_repository_group(&self, path: &str) -> Result<bool, WorkspaceStateError> {
        let file = self.repository_group_file(path);
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(WorkspaceStateError::Message(error.to_string())),
        };
        serde_json::from_slice(&bytes)
            .map_err(|error| WorkspaceStateError::Message(error.to_string()))
    }
    fn save_repository_group(
        &self,
        path: &str,
        collapsed: bool,
    ) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();
        let file = self.repository_group_file(path);
        std::fs::create_dir_all(file.parent().expect("group state directory"))
            .map_err(|error| WorkspaceStateError::Message(error.to_string()))?;
        std::fs::write(file, if collapsed { "true" } else { "false" })
            .map_err(|error| WorkspaceStateError::Message(error.to_string()))
    }

    fn load(
        &self,
        worktree_name: &str,
        worktree_root: &str,
    ) -> Result<Option<WorkspaceState>, crate::domain::workspace_state::WorkspaceStateError> {
        let Some(state) = self.read_state(worktree_name)? else {
            return Ok(None);
        };
        let state = state.into_domain();
        let state = filter_missing_files(state, worktree_root);

        self.entries
            .write()
            .insert(worktree_name.to_string(), state.clone());
        Ok(Some(state))
    }

    fn check_readable(&self, worktree_name: &str) -> Result<(), WorkspaceStateError> {
        self.read_state(worktree_name).map(|_| ())
    }

    fn save(&self, worktree_name: &str) -> Result<(), WorkspaceStateError> {
        let _guard = self.file_lock.lock();

        let dir = state_dir(&self.app_data_dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| WorkspaceStateError::Message(format!("Failed to create dir: {e}")))?;

        let file_path = state_file(&self.app_data_dir, worktree_name);
        let state = {
            let entries = self.entries.read();
            match entries.get(worktree_name) {
                Some(s) => s.clone(),
                None => return Ok(()),
            }
        };
        let json = serde_json::to_string_pretty(&StoredWorkspaceState::from_domain(state))
            .map_err(|e| WorkspaceStateError::Message(format!("Failed to serialize: {e}")))?;
        std::fs::write(&file_path, json)
            .map_err(|e| WorkspaceStateError::Message(format!("Failed to write: {e}")))?;
        Ok(())
    }

    fn set(&self, worktree_name: &str, state: WorkspaceState) {
        self.entries
            .write()
            .insert(worktree_name.to_string(), state);
    }
}

impl WorkspaceStateStore {
    fn repository_group_file(&self, path: &str) -> PathBuf {
        use sha2::Digest;
        let key = hex::encode(sha2::Sha256::digest(
            crate::domain::repository::normalize_repo_path(path).as_bytes(),
        ));
        self.app_data_dir
            .join("repository_group_state")
            .join(format!("{key}.json"))
    }

    fn read_state(
        &self,
        worktree_name: &str,
    ) -> Result<Option<StoredWorkspaceState>, WorkspaceStateError> {
        let data = match std::fs::read_to_string(state_file(&self.app_data_dir, worktree_name)) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !std::path::Path::new(worktree_name).is_absolute() {
                    return Ok(None);
                }
                let legacy = worktree_name.replace(['/', '\\'], "_");
                match std::fs::read_to_string(state_file(&self.app_data_dir, &legacy)) {
                    Ok(data) => data,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(WorkspaceStateError::Message(error.to_string())),
                }
            }
            Err(error) => return Err(WorkspaceStateError::Message(error.to_string())),
        };
        serde_json::from_str::<StoredWorkspaceState>(&data)
            .map(Some)
            .map_err(|error| WorkspaceStateError::Message(error.to_string()))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn get(&self, worktree_name: &str) -> Option<WorkspaceState> {
        self.entries.read().get(worktree_name).cloned()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredWorkspaceState {
    #[serde(flatten)]
    state: WorkspaceStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panes: Option<StoredPaneLayout>,
}
impl StoredWorkspaceState {
    fn from_domain(mut state: WorkspaceState) -> Self {
        let panes = state.panes.take().map(Into::into);
        Self {
            state: state.into(),
            panes,
        }
    }
    fn into_domain(self) -> WorkspaceState {
        let mut state: WorkspaceState = self.state.into();
        state.panes = self.panes.map(Into::into);
        state
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
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSplitAxis {
    Horizontal,
    Vertical,
}

impl From<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>
    for StoredPaneLayout
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
impl From<StoredPaneLayout>
    for crate::domain::workspace_state::value_objects::pane_layout::PaneLayout
{
    fn from(value: StoredPaneLayout) -> Self {
        match value {
            StoredPaneLayout::Pane {
                id,
                tabs,
                active_tab,
            } => Self::Pane {
                id,
                tabs: tabs.into_iter().map(Into::into).collect(),
                active_tab,
            },
            StoredPaneLayout::Split {
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
