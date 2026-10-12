use std::path::Path;

use crate::domain::workspace_state::WorkspaceState;

pub fn filter_missing_files(
    mut state: WorkspaceState,
    exists: impl Fn(&str) -> bool,
) -> WorkspaceState {
    state.tabs.editors.retain(|tab| exists(&tab.path));

    if let Some(ref active) = state.tabs.active_editor_path {
        let still_exists = state.tabs.editors.iter().any(|e| e.path == *active);
        if !still_exists {
            state.tabs.active_editor_path = state.tabs.editors.first().map(|e| e.path.clone());
        }
    }

    if let Some(ref diff_file) = state.layout.selected_diff_file {
        if !exists(diff_file) {
            state.layout.selected_diff_file = None;
        }
    }

    state
}

pub fn migration_worktree_paths(inventories: Vec<Option<Vec<String>>>) -> Vec<String> {
    inventories
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .map(|items| items.into_iter().flatten().collect())
        .unwrap_or_default()
}

pub fn legacy_state_path<'a>(name: &str, candidates: &'a [(String, String)]) -> Option<&'a str> {
    if let Some((path, _)) = candidates.iter().find(|(_, key)| key == name) {
        return Some(path);
    }
    let mut matches = candidates
        .iter()
        .filter(|(path, _)| Path::new(path).file_name().is_some_and(|part| part == name));
    let (first, _) = matches.next()?;
    matches.next().is_none().then_some(first.as_str())
}

#[cfg(test)]
#[path = "services_test.rs"]
mod services_tests;
