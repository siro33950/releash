//! Archive 済みの Session と Workflow の読み取り。
//!
//! どれを出すか、どの順に並べるか、検索語でどう絞るかは集約が決める。

use super::WorkspaceTree;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::search_term::SearchTerm;
use crate::domain::workflow::ExecutionStatus;

/// Archive 済みの一覧の 1 行。Archive 済みの Session か、Archive 済みの Workflow である。
#[derive(Debug, Clone, PartialEq)]
pub enum ArchivedHistoryEntry {
    Session(ArchivedSessionEntry),
    Workflow(ArchivedWorkflowEntry),
}

/// Archive 済みの Session。
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedSessionEntry {
    pub agent_session_id: String,
    pub provider: ProviderKind,
    /// Node 面がその Session に出す名前。
    pub name: String,
    /// Archive した時刻。1970 年からの秒。
    pub archived_at: f64,
    /// 削除できるか。`AgentSession::operations` が決める。
    pub can_delete: bool,
}

/// Archive 済みの Workflow。
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedWorkflowEntry {
    pub execution_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: ExecutionStatus,
    /// 更新時刻。1970 年からの秒。
    pub updated_at: f64,
}

impl WorkspaceTree {
    /// Archive 済みの Session と Workflow を、一つの並びで時刻の新しい順に返す。
    ///
    /// Archive 済みの Session は、Session として起動した実行のうち、Session が Archive 済みのもの。
    /// Archive 済みの Workflow は、Workflow として起動し、この Workspace の worktree で動いた実行のうち、
    /// Archive の記録があるもの。
    ///
    /// 並べる時刻は、Session では Archive した時刻、Workflow では更新時刻である。
    /// 時刻が同じ行は、識別子（Session は Session の ID、Workflow は実行の ID）の順に並べる。
    ///
    /// 検索語があれば、Session は名前、Workflow はタイトルが検索語を含む行に絞る。
    pub fn archived_history(&self, search: Option<&SearchTerm>) -> Vec<ArchivedHistoryEntry> {
        use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
        use crate::domain::workflow::ExecutionTreeLaunch;
        let mut entries = self
            .executions()
            .iter()
            .filter_map(|execution| {
                let archive = execution.archive.as_ref()?;
                match execution.launched_as {
                    ExecutionTreeLaunch::Session => {
                        let session = execution.session.as_ref()?;
                        if session.lifecycle() != AgentSessionLifecycle::Archived {
                            return None;
                        }
                        let node = self.session_node(session.id())?;
                        if search.is_some_and(|term| !term.matches(&node.title)) {
                            return None;
                        }
                        Some(ArchivedHistoryEntry::Session(ArchivedSessionEntry {
                            agent_session_id: session.id().to_owned(),
                            provider: session.provider(),
                            name: node.title.clone(),
                            archived_at: archive.archived_at,
                            can_delete: session.operations().can_delete,
                        }))
                    }
                    ExecutionTreeLaunch::Workflow => {
                        if execution.worktree_path != self.workspace_identity().as_str()
                            || search.is_some_and(|term| !term.matches(&execution.workflow_name))
                        {
                            return None;
                        }
                        Some(ArchivedHistoryEntry::Workflow(ArchivedWorkflowEntry {
                            execution_id: execution.execution_id.clone(),
                            worktree_path: execution.worktree_path.clone(),
                            title: execution.workflow_name.clone(),
                            status: execution.status,
                            updated_at: execution.updated_at,
                        }))
                    }
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            let (left_time, left_id) = history_order(left);
            let (right_time, right_id) = history_order(right);
            right_time
                .total_cmp(&left_time)
                .then_with(|| left_id.cmp(right_id))
        });
        entries
    }
}

fn history_order(entry: &ArchivedHistoryEntry) -> (f64, &str) {
    match entry {
        ArchivedHistoryEntry::Session(value) => (value.archived_at, &value.agent_session_id),
        ArchivedHistoryEntry::Workflow(value) => (value.updated_at, &value.execution_id),
    }
}

#[cfg(test)]
#[path = "archived_history_test.rs"]
mod archived_history_tests;
