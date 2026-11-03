use std::path::Path;

use crate::domain::workspace_state::WorkspaceState;

/// 保存済みの設定から存在しないファイルへの参照を除き、未保存の設定は保持する。
pub fn filter_missing_files(
    mut state: WorkspaceState,
    exists: impl Fn(&str) -> bool,
) -> WorkspaceState {
    if let Some(tabs) = &mut state.tabs {
        tabs.editors.retain(|tab| exists(&tab.path));
        if let Some(active) = &tabs.active_editor_path {
            if !tabs.editors.iter().any(|tab| &tab.path == active) {
                tabs.active_editor_path = tabs.editors.first().map(|tab| tab.path.clone());
            }
        }
    }
    if let Some(layout) = &mut state.layout {
        if layout
            .selected_diff_file
            .as_ref()
            .is_some_and(|path| !exists(path))
        {
            layout.selected_diff_file = None;
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
