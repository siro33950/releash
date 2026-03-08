use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceTabEntry {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTabsState {
    pub editors: Vec<WorkspaceTabEntry>,
    pub active_editor_path: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLayoutState {
    pub center_tab: String,
    pub active_view: String,
    pub left_nav_collapsed: bool,
    pub right_collapsed: bool,
    pub right_bottom_collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_bottom_active_tab: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceState {
    pub version: u32,
    pub tabs: WorkspaceTabsState,
    pub layout: WorkspaceLayoutState,
}

pub struct WorkspaceStateStore {
    entries: RwLock<HashMap<String, WorkspaceState>>,
    file_lock: parking_lot::Mutex<()>,
}

impl Default for WorkspaceStateStore {
    fn default() -> Self {
        Self {
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

impl WorkspaceStateStore {
    pub fn load(
        &self,
        app_data_dir: &Path,
        worktree_name: &str,
        worktree_root: &str,
    ) -> Option<WorkspaceState> {
        let file_path = state_file(app_data_dir, worktree_name);

        if !file_path.exists() {
            return None;
        }

        let data = std::fs::read_to_string(&file_path).ok()?;
        let mut state: WorkspaceState = serde_json::from_str(&data).ok()?;

        // Filter out tabs whose files no longer exist
        state.tabs.editors.retain(|tab| {
            let full_path = if tab.path.starts_with('/') || tab.path.starts_with('\\') {
                PathBuf::from(&tab.path)
            } else {
                Path::new(worktree_root).join(&tab.path)
            };
            full_path.exists()
        });

        // Fix active editor path if it was filtered out
        if let Some(ref active) = state.tabs.active_editor_path {
            let still_exists = state.tabs.editors.iter().any(|e| e.path == *active);
            if !still_exists {
                state.tabs.active_editor_path = state.tabs.editors.first().map(|e| e.path.clone());
            }
        }

        self.entries
            .write()
            .insert(worktree_name.to_string(), state.clone());
        Some(state)
    }

    pub fn save(&self, app_data_dir: &Path, worktree_name: &str) -> Result<(), String> {
        let _guard = self.file_lock.lock();

        let dir = state_dir(app_data_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;

        let file_path = state_file(app_data_dir, worktree_name);
        let state = {
            let entries = self.entries.read();
            match entries.get(worktree_name) {
                Some(s) => s.clone(),
                None => return Ok(()),
            }
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        std::fs::write(&file_path, json).map_err(|e| format!("Failed to write: {e}"))?;
        Ok(())
    }

    pub fn set(&self, worktree_name: &str, state: WorkspaceState) {
        self.entries
            .write()
            .insert(worktree_name.to_string(), state);
    }

    #[cfg(test)]
    pub fn get(&self, worktree_name: &str) -> Option<WorkspaceState> {
        self.entries.read().get(worktree_name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

use tauri::Manager;

#[tauri::command]
pub fn load_workspace_state(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    worktree_root: String,
) -> Option<WorkspaceState> {
    let data_dir = app.path().app_data_dir().ok()?;
    store.load(&data_dir, &worktree_name, &worktree_root)
}

#[tauri::command]
pub fn save_workspace_state(
    app: tauri::AppHandle,
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    state: WorkspaceState,
) -> Result<(), String> {
    store.set(&worktree_name, state);
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))?;
    store.save(&data_dir, &worktree_name)
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let store = WorkspaceStateStore::default();
        let state = make_state();
        store.set("wt1", state.clone());
        store.save(dir.path(), "wt1").unwrap();

        let store2 = WorkspaceStateStore::default();
        let loaded = store2
            .load(dir.path(), "wt1", worktree_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.tabs.editors.len(), 2);
        assert_eq!(
            loaded.tabs.active_editor_path.as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(loaded.layout.center_tab, "editor");
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let dir = TempDir::new().unwrap();
        let store = WorkspaceStateStore::default();
        let result = store.load(dir.path(), "nonexistent", "/tmp");
        assert!(result.is_none());
    }

    #[test]
    fn load_filters_deleted_files() {
        let dir = TempDir::new().unwrap();
        let worktree_dir = dir.path().join("worktree");
        std::fs::create_dir_all(worktree_dir.join("src")).unwrap();
        // Only create main.rs, not lib.rs
        std::fs::write(worktree_dir.join("src/main.rs"), "fn main() {}").unwrap();

        let store = WorkspaceStateStore::default();
        store.set("wt1", make_state());
        store.save(dir.path(), "wt1").unwrap();

        let store2 = WorkspaceStateStore::default();
        let loaded = store2
            .load(dir.path(), "wt1", worktree_dir.to_str().unwrap())
            .unwrap();
        // lib.rs was filtered out
        assert_eq!(loaded.tabs.editors.len(), 1);
        assert_eq!(loaded.tabs.editors[0].path, "src/main.rs");
    }

    #[test]
    fn load_fixes_active_path_when_file_deleted() {
        let dir = TempDir::new().unwrap();
        let worktree_dir = dir.path().join("worktree");
        std::fs::create_dir_all(worktree_dir.join("src")).unwrap();
        // Only create lib.rs — main.rs (the active editor) is missing
        std::fs::write(worktree_dir.join("src/lib.rs"), "// lib").unwrap();

        let store = WorkspaceStateStore::default();
        store.set("wt1", make_state());
        store.save(dir.path(), "wt1").unwrap();

        let store2 = WorkspaceStateStore::default();
        let loaded = store2
            .load(dir.path(), "wt1", worktree_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(loaded.tabs.editors.len(), 1);
        // Active path should fall back to the first remaining tab
        assert_eq!(
            loaded.tabs.active_editor_path.as_deref(),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn load_returns_empty_editors_when_all_files_deleted() {
        let dir = TempDir::new().unwrap();
        let worktree_dir = dir.path().join("worktree");
        // Create src/ directory but not the files referenced in make_state()
        std::fs::create_dir_all(worktree_dir.join("src")).unwrap();

        let store = WorkspaceStateStore::default();
        store.set("wt1", make_state());
        store.save(dir.path(), "wt1").unwrap();

        let store2 = WorkspaceStateStore::default();
        let loaded = store2
            .load(dir.path(), "wt1", worktree_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(loaded.tabs.editors.len(), 0);
        assert_eq!(loaded.tabs.active_editor_path, None);
    }

    #[test]
    fn get_set_in_memory() {
        let store = WorkspaceStateStore::default();
        assert!(store.get("wt1").is_none());

        store.set("wt1", make_state());
        let got = store.get("wt1").unwrap();
        assert_eq!(got.tabs.editors.len(), 2);
    }
}
