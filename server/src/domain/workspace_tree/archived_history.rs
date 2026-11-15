//! Archive 済みの実行の読み取り。
//!
//! どれを出すか、どの順に並べるか、検索語でどう絞るかは集約が決める。

use super::{WorkspaceExecution, WorkspaceTree};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionLifecycle};
use crate::domain::search_term::SearchTerm;
use crate::domain::workflow::ExecutionTreeLaunch;

/// Archive 済みの実行。
#[derive(Debug, Clone, PartialEq)]
pub enum ArchivedExecution {
    /// Session として起動した実行。
    Session {
        session: AgentSession,
        /// Session の名前。
        name: String,
        /// Archive した時刻。1970 年からの秒。
        archived_at: f64,
    },
    /// Workflow として起動した実行。
    Workflow(WorkspaceExecution),
}

impl ArchivedExecution {
    /// 識別子。Session として起動した実行は Session の ID、Workflow として起動した実行は実行の ID である。
    /// 時刻が同じ実行の並びを決める。
    pub fn id(&self) -> &str {
        match self {
            Self::Session { session, .. } => session.id(),
            Self::Workflow(execution) => &execution.execution_id,
        }
    }

    /// 名前。Session として起動した実行は Session の名前、Workflow として起動した実行はタイトルである。
    /// 検索語はこの名前に当てる。
    pub fn name(&self) -> &str {
        match self {
            Self::Session { name, .. } => name,
            Self::Workflow(execution) => &execution.workflow_name,
        }
    }

    /// 並べる時刻。1970 年からの秒。
    /// Session として起動した実行は Archive した時刻、Workflow として起動した実行は更新時刻である。
    pub fn ordered_at(&self) -> f64 {
        match self {
            Self::Session { archived_at, .. } => *archived_at,
            Self::Workflow(execution) => execution.updated_at,
        }
    }
}

impl WorkspaceTree {
    /// Archive 済みの実行を、一つの並びで時刻の新しい順に返す。
    ///
    /// Session として起動した実行は、Session が Archive 済みのものを返す。
    /// Workflow として起動した実行は、この Workspace の worktree で動き、Archive の記録があるものを返す。
    ///
    /// 並べる時刻は `ArchivedExecution::ordered_at` が決める。
    /// 時刻が同じ実行は、`ArchivedExecution::id` の順に並べる。
    ///
    /// 検索語があれば、`ArchivedExecution::name` が検索語を含む実行に絞る。
    pub fn archived_history(&self, search: Option<&SearchTerm>) -> Vec<ArchivedExecution> {
        let mut entries = self
            .executions()
            .iter()
            .filter_map(|execution| {
                let archive = execution.archive.as_ref()?;
                let entry = match execution.launched_as {
                    ExecutionTreeLaunch::Session => {
                        let session = execution.session.as_ref()?;
                        if session.lifecycle() != AgentSessionLifecycle::Archived {
                            return None;
                        }
                        ArchivedExecution::Session {
                            session: session.clone(),
                            name: self.session_node(session.id())?.title.clone(),
                            archived_at: archive.archived_at,
                        }
                    }
                    ExecutionTreeLaunch::Workflow => {
                        if execution.worktree_path != self.workspace_identity().as_str() {
                            return None;
                        }
                        ArchivedExecution::Workflow(execution.clone())
                    }
                };
                if search.is_some_and(|term| !term.matches(entry.name())) {
                    return None;
                }
                Some(entry)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .ordered_at()
                .total_cmp(&left.ordered_at())
                .then_with(|| left.id().cmp(right.id()))
        });
        entries
    }
}

#[cfg(test)]
#[path = "archived_history_test.rs"]
mod archived_history_tests;
