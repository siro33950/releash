use std::collections::HashMap;

use super::branch_state::{Continuation, Operation, OperationKind, Push, WorktreeBranch};
use crate::domain::git::entities::file_status::{ChangeKind, ConflictKind};
use crate::domain::git::{CommitMessage, GitError, RepositoryStatusScan};
use crate::domain::search_term::SearchTerm;

/// worktree の Git の状態。ブランチの状態と、変更したファイルを持つ。操作を受け付けるかは、この状態が決める。
/// 変更の行数は持たない。行数は `LineChanges` が持つ。
///
/// 変更したファイルは、conflict しているもの、stage 済みの変更、まだ stage していない変更に分けて持つ。
/// 同じファイルに stage 済みの変更とまだ stage していない変更があるとき、そのファイルは両方にある。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeStatus {
    branch: WorktreeBranch,
    changed_file_count: usize,
    conflicts: Vec<ConflictedFile>,
    staged: Vec<FileChange>,
    unstaged: Vec<FileChange>,
    search: Option<SearchTerm>,
}

/// 変更の一覧の 1 行。stage 済みか、まだ stage していないかの、どちらか一方の側での 1 ファイルの変更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// worktree からの相対 path。
    pub path: String,
    pub kind: ChangeKind,
}

/// conflict しているファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictedFile {
    /// worktree からの相対 path。
    pub path: String,
    pub kind: ConflictKind,
}

/// 追加した行数と削除した行数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCount {
    pub additions: u32,
    pub deletions: u32,
}

/// ファイルごとの変更の行数。キーは worktree からの相対 path。表示にだけ使う。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineChanges {
    /// stage 済みの変更の行数。
    pub staged: HashMap<String, LineCount>,
    /// まだ stage していない変更の行数。git が追跡していないファイルは、全部の行を追加として数える。
    pub unstaged: HashMap<String, LineCount>,
}

impl LineChanges {
    /// ファイルの、stage 済みの変更の行数を返す。行数を持っていないファイルは 0 である。
    pub fn staged_lines(&self, path: &str) -> LineCount {
        self.staged.get(path).copied().unwrap_or_default()
    }

    /// ファイルの、まだ stage していない変更の行数を返す。行数を持っていないファイルは 0 である。
    pub fn unstaged_lines(&self, path: &str) -> LineCount {
        self.unstaged.get(path).copied().unwrap_or_default()
    }
}

/// まだ stage していない変更の破棄のしかた。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscardPlan {
    /// index の内容に戻すファイル。
    pub restore: Vec<String>,
    /// OS のゴミ箱に移すファイル。
    pub trash: Vec<String>,
}

/// まだ stage していない変更をまとめて破棄したときに起きること。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiscardSummary {
    /// 消える行数の合計。index の内容に戻すファイルの分だけを数える。
    pub lines: LineCount,
    /// OS のゴミ箱に移すファイルの数。
    pub trashed_files: usize,
}

impl WorktreeStatus {
    fn matches(&self, path: &str) -> bool {
        self.search
            .as_ref()
            .is_none_or(|search| search.matches(path.rsplit('/').next().unwrap_or(path)))
    }

    /// ブランチの状態と変更の走査結果から、worktree の Git の状態を作る。
    ///
    /// - 走査結果のファイルを、`FileStatus::conflict`、`FileStatus::staged_change`、`FileStatus::unstaged_change` で分ける。
    /// - 変更したファイルの数は、走査結果の `dirty_count` である。
    /// - conflict しているファイル、stage 済みの変更、まだ stage していない変更は、それぞれ path の順に並べる。
    pub fn new(branch: WorktreeBranch, scan: RepositoryStatusScan) -> Self {
        let mut status = Self {
            branch,
            changed_file_count: scan.dirty_count,
            conflicts: Vec::new(),
            staged: Vec::new(),
            unstaged: Vec::new(),
            search: None,
        };
        for file in scan.status {
            if let Some(kind) = file.conflict {
                status.conflicts.push(ConflictedFile {
                    path: file.path.clone(),
                    kind,
                });
            }
            if let Some(kind) = file.staged_change() {
                status.staged.push(FileChange {
                    path: file.path.clone(),
                    kind,
                });
            }
            if let Some(kind) = file.unstaged_change() {
                status.unstaged.push(FileChange {
                    path: file.path.clone(),
                    kind,
                });
            }
        }
        status.conflicts.sort_by(|a, b| a.path.cmp(&b.path));
        status.staged.sort_by(|a, b| a.path.cmp(&b.path));
        status.unstaged.sort_by(|a, b| a.path.cmp(&b.path));
        status
    }

    /// 一覧を、ファイル名が検索語を含む行に絞る。ファイル名は path の最後の要素である。
    ///
    /// 絞るのは `conflicts`、`staged`、`unstaged`、`discard_summary` が返すものである。
    /// 数（`changed_file_count`、`conflict_count`）と、操作を受け付けるかは、絞り込む前の全部から決まる。
    /// `None` を渡すと絞り込まない。
    pub fn searching(self, search: Option<SearchTerm>) -> Self {
        Self { search, ..self }
    }

    pub fn branch(&self) -> &WorktreeBranch {
        &self.branch
    }

    /// 変更したファイルの数。
    pub fn changed_file_count(&self) -> usize {
        self.changed_file_count
    }

    /// conflict しているファイルの数。絞り込む前の数である。
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// conflict しているファイルの一覧。
    pub fn conflicts(&self) -> Vec<&ConflictedFile> {
        self.conflicts
            .iter()
            .filter(|file| self.matches(&file.path))
            .collect()
    }

    /// stage 済みの変更の一覧。
    pub fn staged(&self) -> Vec<&FileChange> {
        self.staged
            .iter()
            .filter(|file| self.matches(&file.path))
            .collect()
    }

    /// まだ stage していない変更の一覧。
    pub fn unstaged(&self) -> Vec<&FileChange> {
        self.unstaged
            .iter()
            .filter(|file| self.matches(&file.path))
            .collect()
    }

    /// commit を受け付けるかを決める。
    ///
    /// stage 済みの変更が 1 つも無いときと、merge か rebase の途中のときは、受け付けない。
    pub fn ensure_can_commit(&self) -> Result<(), GitError> {
        if self.staged.is_empty()
            || matches!(
                self.branch.state,
                super::branch_state::BranchState::Operating(_)
            )
        {
            return Err(GitError::rule(
                "Cannot commit without staged changes or during an operation",
            ));
        }
        Ok(())
    }

    /// 進行中の merge か rebase の続行を受け付けるかを決め、受け付けるなら進行中の操作の種類を返す。
    ///
    /// merge か rebase の途中でないときと、conflict しているファイルが残っているときは、受け付けない。
    pub fn ensure_can_continue(&self) -> Result<&OperationKind, GitError> {
        Ok(&self.continuable_operation()?.kind)
    }

    fn continuable_operation(&self) -> Result<&Operation, GitError> {
        let super::branch_state::BranchState::Operating(operation) = &self.branch.state else {
            return Err(GitError::rule("No merge or rebase in progress"));
        };
        if !self.conflicts.is_empty() {
            return Err(GitError::rule("Resolve all conflicts before continuing"));
        }
        Ok(operation)
    }

    /// 進行中の merge か rebase の続行を受け付けるかを決め、受け付けるなら続け方を返す。
    ///
    /// - 受け付けるかは `ensure_can_continue` と同じである。
    /// - merge の続行は、必ず commit を作る。入力の前後の空白を除いたメッセージで作る。空なら失敗する。
    ///   merge の途中なのに `Operation::message` が無いときも失敗する。
    /// - rebase の続行が commit を作るとき（`Operation::message` があるとき）は、入力の前後の空白を除いた
    ///   メッセージで作る。空なら失敗する。
    /// - rebase の続行が commit を作らないとき（`Operation::message` が無いとき）は、入力を見ない。
    pub fn continuation(&self, input: &str) -> Result<Continuation, GitError> {
        let operation = self.continuable_operation()?;
        match (&operation.kind, &operation.message) {
            (OperationKind::Merge, Some(_)) => Ok(Continuation::Merge(CommitMessage::new(input)?)),
            (OperationKind::Merge, None) => Err(GitError::rule("Missing merge commit message")),
            (OperationKind::Rebase(_), Some(_)) => {
                Ok(Continuation::Rebase(Some(CommitMessage::new(input)?)))
            }
            (OperationKind::Rebase(_), None) => Ok(Continuation::Rebase(None)),
        }
    }

    /// 進行中の merge か rebase の中止を受け付けるかを決め、受け付けるなら進行中の操作の種類を返す。
    ///
    /// merge か rebase の途中でないときは、受け付けない。
    pub fn ensure_can_abort(&self) -> Result<&OperationKind, GitError> {
        match &self.branch.state {
            super::branch_state::BranchState::Operating(operation) => Ok(&operation.kind),
            _ => Err(GitError::rule("No merge or rebase in progress")),
        }
    }

    /// ブランチの commit の、origin への送り方を決める。
    ///
    /// - upstream が無いブランチは、`force_with_lease` に関わらず `Publish` である。
    /// - upstream があるブランチは、`force_with_lease` が真なら `ForceWithLease`、偽なら `ToUpstream` である。
    /// - merge か rebase の途中のときは、受け付けない。
    pub fn push_plan(&self, force_with_lease: bool) -> Result<Push, GitError> {
        match self.branch.state {
            super::branch_state::BranchState::Unpublished { .. } => Ok(Push::Publish),
            super::branch_state::BranchState::Tracking(_) => Ok(if force_with_lease {
                Push::ForceWithLease
            } else {
                Push::ToUpstream
            }),
            super::branch_state::BranchState::Operating(_) => {
                Err(GitError::rule("Cannot push during merge or rebase"))
            }
        }
    }

    /// 指定したファイルの、まだ stage していない変更の破棄のしかたを決める。
    ///
    /// - git が追跡していないファイル（`Untracked`）は OS のゴミ箱に移し、ほかのファイルは index の内容に戻す。
    /// - `paths` が空のときと、まだ stage していない変更が無いファイルが `paths` にあるときは、受け付けない。
    /// - 対象は絞り込む前の全部から探す。
    pub fn discard_plan(&self, paths: &[String]) -> Result<DiscardPlan, GitError> {
        if paths.is_empty() {
            return Err(GitError::rule("No files selected for discard"));
        }
        let mut plan = DiscardPlan::default();
        for path in paths {
            let file = self
                .unstaged
                .iter()
                .find(|file| &file.path == path)
                .ok_or_else(|| GitError::rule(format!("No unstaged change: {path}")))?;
            let target = if file.kind == ChangeKind::Untracked {
                &mut plan.trash
            } else {
                &mut plan.restore
            };
            if !target.contains(path) {
                target.push(path.clone());
            }
        }
        Ok(plan)
    }

    /// `unstaged` が返す全部を破棄したときに起きることを返す。消える行数は `lines` から引く。
    pub fn discard_summary(&self, lines: &LineChanges) -> DiscardSummary {
        let mut summary = DiscardSummary::default();
        for file in self.unstaged() {
            if file.kind == ChangeKind::Untracked {
                summary.trashed_files += 1;
            } else {
                let count = lines.unstaged_lines(&file.path);
                summary.lines.additions = summary.lines.additions.saturating_add(count.additions);
                summary.lines.deletions = summary.lines.deletions.saturating_add(count.deletions);
            }
        }
        summary
    }
}

#[cfg(test)]
#[path = "worktree_status_test.rs"]
mod worktree_status_tests;
