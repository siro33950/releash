use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};

/// worktree のタブ配置と選択状態。
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceState {
    pub panes: Option<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>,
    pub version: u32,
    pub tabs: Option<WorkspaceTabsState>,
    pub layout: Option<WorkspaceLayoutState>,
}

impl WorkspaceState {
    /// 空のタブ配置を作る。エディタ設定とパネル設定は未保存とする。
    pub fn new() -> Self {
        todo!()
    }

    /// Session のタブを、既存の選択を変えずに追加する。
    pub fn add_session_tab(&mut self, _session_id: &str) {
        todo!()
    }

    /// 指定された Session のタブを削除する。
    pub fn remove_session_tab(&mut self, _session_id: &str) {
        todo!()
    }

    /// 実行木に属する Session のタブを、既存の選択を変えずに追加する。
    pub fn add_execution_tabs(&mut self, _tree: &crate::domain::workspace_tree::WorkspaceTree) {
        todo!()
    }

    /// 実行木に属する Session のタブを削除する。
    pub fn remove_execution_tabs(&mut self, _tree: &crate::domain::workspace_tree::WorkspaceTree) {
        todo!()
    }

    /// 配置が未保存なら空の配置を持つ状態にする。保存済みの配置と設定は保持する。
    pub fn initialize_panes(self) -> Self {
        todo!()
    }

    /// 更新で省略された配置と設定を、保存済みの状態から引き継ぐ。
    pub fn preserve_omitted_fields(self, _previous: Option<Self>) -> Self {
        todo!()
    }
}

#[cfg(test)]
#[path = "workspace_state_test.rs"]
mod workspace_state_tests;
