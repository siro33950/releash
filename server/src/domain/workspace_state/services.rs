use crate::domain::workspace_state::WorkspaceState;

/// 保存済みの設定から存在しないファイルへの参照を除き、未保存の設定は保持する。
pub fn filter_missing_files(
    mut state: WorkspaceState,
    exists: impl Fn(&str) -> bool,
) -> WorkspaceState {
    if let Some(tabs) = &mut state.tabs {
        tabs.editors.retain(|tab| exists(&tab.path));
    }
    state
}

#[cfg(test)]
#[path = "services_test.rs"]
mod services_tests;
