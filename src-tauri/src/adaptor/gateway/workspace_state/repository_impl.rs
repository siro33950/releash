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

fn state_file(app_data_dir: &Path, worktree_name: &str) -> PathBuf {
    let safe_name = worktree_name.replace(['/', '\\'], "_");
    state_dir(app_data_dir).join(format!("{safe_name}.json"))
}

impl WorkspaceStateRepository for WorkspaceStateStore {
    fn load(&self, worktree_name: &str, worktree_root: &str) -> Option<WorkspaceState> {
        let file_path = state_file(&self.app_data_dir, worktree_name);

        if !file_path.exists() {
            return None;
        }

        let data = std::fs::read_to_string(&file_path).ok()?;
        let state: WorkspaceState = serde_json::from_str::<WorkspaceStateDto>(&data)
            .ok()
            .map(WorkspaceState::from)?;
        let state = filter_missing_files(state, worktree_root);

        self.entries
            .write()
            .insert(worktree_name.to_string(), state.clone());
        Some(state)
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
    #[cfg(test)]
    pub fn get(&self, worktree_name: &str) -> Option<WorkspaceState> {
        self.entries.read().get(worktree_name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace_state::value_objects::{
        workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
    };
    use tempfile::TempDir;

    fn make_state() -> WorkspaceState {
        WorkspaceState {
            version: 1,
            tabs: WorkspaceTabsState {
                editors: vec![
                    WorkspaceTabEntry {
                        path: "src/main.rs".to_string(),
                        name: "main.rs".to_string(),
                    },
                    WorkspaceTabEntry {
                        path: "src/lib.rs".to_string(),
                        name: "lib.rs".to_string(),
                    },
                ],
                active_editor_path: Some("src/main.rs".to_string()),
            },
            layout: WorkspaceLayoutState {
                center_tab: "editor".to_string(),
                active_view: "git".to_string(),
                left_nav_collapsed: false,
                right_collapsed: false,
                right_bottom_collapsed: false,
                right_bottom_active_tab: None,
                selected_diff_file: None,
            },
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let worktree_dir = dir.path().join("worktree");
        std::fs::create_dir_all(worktree_dir.join("src")).unwrap();
        std::fs::write(worktree_dir.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(worktree_dir.join("src/lib.rs"), "// lib").unwrap();

        let store = WorkspaceStateStore::new(dir.path().to_path_buf());
        store.set("wt1", make_state());
        store.save("wt1").unwrap();

        let loaded = store.load("wt1", worktree_dir.to_str().unwrap()).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.tabs.editors.len(), 2);
        assert_eq!(loaded.layout.center_tab, "editor");
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = WorkspaceStateStore::new(dir.path().to_path_buf());
        assert!(store.load("nonexistent", "/tmp").is_none());
    }

    #[test]
    fn get_set_in_memory() {
        let dir = TempDir::new().unwrap();
        let store = WorkspaceStateStore::new(dir.path().to_path_buf());
        assert!(store.get("wt1").is_none());
        store.set("wt1", make_state());
        assert_eq!(store.get("wt1").unwrap().tabs.editors.len(), 2);
    }

    #[test]
    fn save_returns_workspace_state_error_when_state_dir_cannot_be_created() {
        let dir = TempDir::new().unwrap();
        let app_data_file = dir.path().join("app-data");
        std::fs::write(&app_data_file, "not a directory").unwrap();

        let store = WorkspaceStateStore::new(app_data_file);
        store.set("wt1", make_state());

        let err = store.save("wt1").unwrap_err();
        assert!(matches!(err, WorkspaceStateError::Message(_)));
        assert!(err.to_string().contains("Failed to create dir"));
    }
}
