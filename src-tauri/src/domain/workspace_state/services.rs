use std::path::Path;

use crate::domain::workspace_state::WorkspaceState;

fn path_exists(worktree_root: &str, path: &str) -> bool {
    let path = Path::new(path);
    let full_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(worktree_root).join(path)
    };
    full_path.exists()
}

pub fn filter_missing_files(mut state: WorkspaceState, worktree_root: &str) -> WorkspaceState {
    state
        .tabs
        .editors
        .retain(|tab| path_exists(worktree_root, &tab.path));

    if let Some(ref active) = state.tabs.active_editor_path {
        let still_exists = state.tabs.editors.iter().any(|e| e.path == *active);
        if !still_exists {
            state.tabs.active_editor_path = state.tabs.editors.first().map(|e| e.path.clone());
        }
    }

    if let Some(ref diff_file) = state.layout.selected_diff_file {
        if !path_exists(worktree_root, diff_file) {
            state.layout.selected_diff_file = None;
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace_state::value_objects::{
        workspace_tabs_state::WorkspaceTabEntry, WorkspaceLayoutState, WorkspaceTabsState,
    };

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
                selected_diff_file: Some("src/deleted.rs".to_string()),
            },
        }
    }

    #[test]
    fn filters_deleted_files_and_falls_back_active_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(worktree.join("src/lib.rs"), "// lib").unwrap();

        let state = filter_missing_files(make_state(), worktree.to_str().unwrap());

        assert_eq!(state.tabs.editors.len(), 1);
        assert_eq!(state.tabs.active_editor_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(state.layout.selected_diff_file, None);
    }
}
