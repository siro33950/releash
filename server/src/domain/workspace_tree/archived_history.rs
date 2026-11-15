//! Archive 済みの実行の読み取り。
//!
//! どれを出すか、どの順に並べるか、検索語でどう絞るかは集約が決める。

use super::{WorkspaceExecution, WorkspaceTree};
use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
use crate::domain::search_term::SearchTerm;
use crate::domain::workflow::ExecutionTreeLaunch;

/// Archive 済みの実行。Session として起動した実行か、Workflow として起動した実行である。
#[derive(Debug, Clone, Copy)]
pub struct ArchivedExecution<'a> {
    execution: &'a WorkspaceExecution,
    name: &'a str,
}

impl<'a> ArchivedExecution<'a> {
    pub fn execution(&self) -> &'a WorkspaceExecution {
        self.execution
    }

    /// 名前。Session として起動した実行は Session の名前、Workflow として起動した実行はタイトルである。
    /// 検索語はこの名前に当てる。
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// 並べる時刻。1970 年からの秒。
    /// Session として起動した実行は Archive した時刻、Workflow として起動した実行は更新時刻である。
    pub fn ordered_at(&self) -> f64 {
        match self.execution.launched_as {
            ExecutionTreeLaunch::Session => {
                self.execution
                    .archive
                    .as_ref()
                    .expect("archived execution")
                    .archived_at
            }
            ExecutionTreeLaunch::Workflow => self.execution.updated_at,
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
    /// 時刻が同じ実行は、識別子（Session として起動した実行は Session の ID、
    /// Workflow として起動した実行は実行の ID）の順に並べる。
    ///
    /// 検索語があれば、`ArchivedExecution::name` が検索語を含む実行に絞る。
    pub fn archived_history(&self, search: Option<&SearchTerm>) -> Vec<ArchivedExecution<'_>> {
        let mut entries = self
            .executions()
            .iter()
            .filter_map(|execution| {
                execution.archive.as_ref()?;
                let name = match execution.launched_as {
                    ExecutionTreeLaunch::Session => {
                        let session = execution.session.as_ref()?;
                        if session.lifecycle() != AgentSessionLifecycle::Archived {
                            return None;
                        }
                        self.session_node(session.id())?.title.as_str()
                    }
                    ExecutionTreeLaunch::Workflow => {
                        if execution.worktree_path != self.workspace_identity().as_str() {
                            return None;
                        }
                        execution.workflow_name.as_str()
                    }
                };
                if search.is_some_and(|term| !term.matches(name)) {
                    return None;
                }
                Some(ArchivedExecution { execution, name })
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            right
                .ordered_at()
                .total_cmp(&left.ordered_at())
                .then_with(|| history_id(left).cmp(history_id(right)))
        });
        entries
    }
}

fn history_id<'a>(entry: &ArchivedExecution<'a>) -> &'a str {
    match entry.execution.launched_as {
        ExecutionTreeLaunch::Session => entry
            .execution
            .session
            .as_ref()
            .expect("archived session")
            .id(),
        ExecutionTreeLaunch::Workflow => &entry.execution.execution_id,
    }
}

#[cfg(test)]
#[path = "archived_history_test.rs"]
mod archived_history_tests;
