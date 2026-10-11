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

pub fn legacy_worktree_path<'a>(name: &str, paths: &'a [String]) -> Option<&'a str> {
    let mut matches = paths
        .iter()
        .filter(|path| Path::new(path).file_name().is_some_and(|part| part == name));
    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}

#[cfg(test)]
#[path = "services_test.rs"]
mod services_tests;
