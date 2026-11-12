use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycleEvent};
use crate::domain::workspace_state::value_objects::pane_layout::PaneLayout;
use crate::domain::workspace_state::value_objects::{WorkspaceLayoutState, WorkspaceTabsState};

/// worktree のタブ配置と選択状態。
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceState {
    pub panes: Option<crate::domain::workspace_state::value_objects::pane_layout::PaneLayout>,
    pub version: u32,
    pub tabs: Option<WorkspaceTabsState>,
    pub layout: Option<WorkspaceLayoutState>,
}

/// 空のタブ配置。エディタ設定とパネル設定は未保存とする。
impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            panes: Some(empty_panes()),
            version: 1,
            tabs: None,
            layout: None,
        }
    }
}

impl WorkspaceState {
    /// 同じ Node の Session 未確定の Session タブに関連付け、無ければ追加する。既存の選択は変えない。
    pub fn add_session_tab(&mut self, session_id: &str, node_id: &str) {
        let panes = self.panes.get_or_insert_with(empty_panes);
        panes.add_session_tab(session_id, node_id);
    }

    /// 指定された Session のタブを削除する。
    pub fn remove_session_tab(&mut self, session_id: &str) {
        if let Some(panes) = &mut self.panes {
            panes.remove_session_tab(Some(session_id), None);
        }
    }

    /// Session に起きた事実にタブ配置を追従させる。
    /// 停止の事実（`ProcessExited`）では、その Session のタブを閉じる。
    /// 再開の事実（`Resumed`）では、その Session のタブを `add_session_tab` と同じ規則で開く。
    /// そのほかの事実では配置を変えない。
    pub fn follow_session(&mut self, session: &AgentSession, event: &AgentSessionLifecycleEvent) {
        match session_tab_change(event) {
            Some(SessionTabChange::Close) => {
                self.remove_session_tab(session.id());
            }
            Some(SessionTabChange::Open) => {
                self.add_session_tab(session.id(), session.tree_location().node_execution_id());
            }
            None => {}
        }
    }

    /// Session に起きた事実のうち、`follow_session` がタブ配置を変えるものか。
    /// 偽を返す事実だけなら、タブ配置を読み込まずに済ませてよい。
    pub fn follows_session_event(event: &AgentSessionLifecycleEvent) -> bool {
        session_tab_change(event).is_some()
    }

    /// 実行木に属する Session のタブを、既存の選択を変えずに追加する。
    pub fn add_execution_tabs(&mut self, tree: &crate::domain::workspace_tree::WorkspaceTree) {
        for node in tree.nodes().iter().filter(|node| !node.is_retry_history) {
            if let Some(session_id) = &node.session_id {
                self.add_session_tab(session_id, &node.id);
            }
        }
    }

    /// 実行木に属する Session のタブを削除する。
    pub fn remove_execution_tabs(&mut self, tree: &crate::domain::workspace_tree::WorkspaceTree) {
        for node in tree.nodes() {
            if let Some(panes) = &mut self.panes {
                panes.remove_session_tab(node.session_id.as_deref(), Some(&node.id));
            }
        }
    }

    /// 配置が未保存なら空の配置を持つ状態にする。保存済みの配置と設定は保持する。
    pub fn initialize_panes(mut self) -> Self {
        self.panes.get_or_insert_with(empty_panes);
        self
    }

    /// 更新で省略された配置と設定を、保存済みの状態から引き継ぐ。
    pub fn preserve_omitted_fields(mut self, previous: Option<Self>) -> Self {
        if let Some(previous) = previous {
            self.panes = self.panes.or(previous.panes);
            self.tabs = self.tabs.or(previous.tabs);
            self.layout = self.layout.or(previous.layout);
        }
        self
    }
}

/// Session に起きた事実が Session タブに及ぼす変更。
enum SessionTabChange {
    /// その Session のタブを閉じる。
    Close,
    /// その Session のタブを開く。
    Open,
}

/// Session に起きた事実と Session タブの変更の対応。変更の無い事実には `None` を返す。
fn session_tab_change(event: &AgentSessionLifecycleEvent) -> Option<SessionTabChange> {
    match event {
        AgentSessionLifecycleEvent::ProcessExited { .. } => Some(SessionTabChange::Close),
        AgentSessionLifecycleEvent::Resumed => Some(SessionTabChange::Open),
        _ => None,
    }
}

fn empty_panes() -> PaneLayout {
    PaneLayout::pane("pane".into(), vec![], None).expect("empty pane is valid")
}

#[cfg(test)]
#[path = "workspace_state_test.rs"]
mod workspace_state_tests;
