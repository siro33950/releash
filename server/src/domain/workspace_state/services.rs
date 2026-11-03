use std::path::Path;

use crate::domain::workspace_state::WorkspaceState;

/// 保存済みの設定から存在しないファイルへの参照を除き、未保存の設定は保持する。
pub fn filter_missing_files(
    _state: WorkspaceState,
    _exists: impl Fn(&str) -> bool,
) -> WorkspaceState {
    todo!()
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
