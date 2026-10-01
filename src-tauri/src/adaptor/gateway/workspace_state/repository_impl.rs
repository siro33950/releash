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

pub(crate) fn storage_key(worktree_name: &str) -> String {
    worktree_name.replace(['/', '\\'], "_")
}

fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = storage_key(worktree_name);
    state_dir(app_data_dir).join(format!("{safe_name}.json"))
}

impl WorkspaceStateRepository for WorkspaceStateStore {
    fn load(
        &self,
        worktree_name: &str,
        worktree_root: &str,
    ) -> Result<Option<WorkspaceState>, crate::domain::workspace_state::WorkspaceStateError> {
        let Some(state) = self.read_state(worktree_name)? else {
            return Ok(None);
        };
        let state = WorkspaceState::from(state);
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
        let json = serde_json::to_string_pretty(&WorkspaceStateDto::from(state))
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
    fn read_state(
        &self,
        worktree_name: &str,
    ) -> Result<Option<WorkspaceStateDto>, WorkspaceStateError> {
        let data = match std::fs::read_to_string(state_file(&self.app_data_dir, worktree_name)) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(WorkspaceStateError::Message(error.to_string())),
        };
        serde_json::from_str::<WorkspaceStateDto>(&data)
            .map(Some)
            .map_err(|error| WorkspaceStateError::Message(error.to_string()))
    }

    #[cfg(test)]
    pub fn get(&self, worktree_name: &str) -> Option<WorkspaceState> {
        self.entries.read().get(worktree_name).cloned()
    }
}

#[cfg(test)]
#[path = "repository_impl_test.rs"]
mod repository_impl_tests;
