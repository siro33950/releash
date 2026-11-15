//! Git の操作と状態の読み取りの入口。
//!
//! controller / watcher / workflow はこの Usecase だけを入口とする。
//! 書き込み・複数集約のオーケストレーションに加え、Entity をそのまま返す読み取りも
//! ドメインサービスへ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::git::{
    worktree_path as derive_worktree_path, Branch, CommitMessage, GitError, GitService,
    RepoLocator, RepositoryStatusScan, Worktree, WorktreeStatus, WorktreeTerminalGateway,
};
use crate::domain::path::to_canonical_forward_slash;

use super::repository_dto::WorktreeEntryDto;
use super::repository_error::UsecaseError;
use super::worktree_operation::WorktreeOperations;

#[async_trait::async_trait]
pub trait WorktreeExecutionArchiver: Send + Sync {
    async fn begin_worktree_deletion(
        &self,
        worktree_path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeDeletionGuard,
        crate::domain::workflow::WorkflowError,
    >;
    async fn archive_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError>;
}

/// Git のブランチと worktree の操作を受け付ける。
#[derive(Clone)]
pub struct GitUsecase {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    git: Arc<dyn GitService>,
    locator: Arc<dyn RepoLocator>,
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    worktree_operations: Arc<WorktreeOperations>,
}

impl GitUsecase {
    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub fn worktree_operations(&self) -> Arc<WorktreeOperations> {
        self.worktree_operations.clone()
    }

    /// Git と作業場所の参照先、worktree の操作を受け取る。
    pub fn new(
        git: Arc<dyn GitService>,
        locator: Arc<dyn RepoLocator>,
        worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
        worktree_operations: Arc<WorktreeOperations>,
    ) -> Self {
        Self {
            state_publisher: None,
            git,
            locator,
            worktree_terminals,
            worktree_operations,
        }
    }

    fn notify_repository_changed(&self, path: &str) {
        if let Some(publisher) = &self.state_publisher {
            publisher.notify(
                crate::usecase::state_subscription::StateChangeSource::Repository(
                    vec![path.into()],
                ),
            );
        }
    }

    // ── branch（読み取り） ──

    pub fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, UsecaseError> {
        Ok(self.git.list_branches(repo_path)?)
    }

    /// 新しい worktree のブランチを作る元にできるブランチと、デフォルトブランチを返す。
    /// ブランチは、ローカル、origin のリモートの順に、それぞれ名前順で並べる。
    pub fn worktree_bases(
        &self,
        repo_path: &str,
    ) -> Result<(Vec<Branch>, Option<String>), UsecaseError> {
        let mut branches: Vec<_> = self
            .git
            .list_branches(repo_path)?
            .into_iter()
            .filter(Branch::is_worktree_base)
            .collect();
        branches.sort_by(|left, right| {
            (left.is_remote, &left.name).cmp(&(right.is_remote, &right.name))
        });
        Ok((branches, self.git.default_branch(repo_path)?))
    }

    // ── branch（書き込み） ──

    pub fn create_branch(&self, repo_path: &str, branch_name: &str) -> Result<(), UsecaseError> {
        self.git.create_branch(repo_path, branch_name)?;
        Ok(())
    }

    pub fn get_repository_status_scan(
        &self,
        repo_path: &str,
    ) -> Result<RepositoryStatusScan, UsecaseError> {
        Ok(self.git.status_scan(repo_path)?)
    }

    // ── worktree の Git の状態（読み取り） ──

    /// 変更の走査結果に、ブランチの状態と変更の行数を合わせて、worktree の Git の状態を返す。
    pub fn worktree_status(
        &self,
        worktree: &str,
        scan: RepositoryStatusScan,
    ) -> Result<WorktreeStatus, UsecaseError> {
        let branch = self.git.worktree_branch(worktree)?;
        let lines = self.git.line_changes(worktree)?;
        Ok(WorktreeStatus::new(branch, scan, lines))
    }

    /// worktree を今走査して、Git の状態を返す。操作を受け付けるかは、この状態で決める。
    fn current_worktree_status(&self, worktree: &str) -> Result<WorktreeStatus, UsecaseError> {
        let scan = self.git.status_scan(worktree)?;
        self.worktree_status(worktree, scan)
    }

    // ── commit・push・pull・fetch（書き込み） ──

    /// stage 済みの変更を、メッセージを付けて commit にする。
    pub fn commit(&self, worktree: &str, message: &str) -> Result<(), UsecaseError> {
        let message = CommitMessage::new(message)?;
        self.current_worktree_status(worktree)?
            .ensure_can_commit()?;
        self.git.commit(worktree, &message)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    /// commit の後に Push する。commit が失敗したら Push しない。
    /// Push が失敗したときは、commit を作った後の失敗（`UsecaseError::AfterCommit`）を返す。
    pub fn commit_and_push(&self, worktree: &str, message: &str) -> Result<(), UsecaseError> {
        self.commit(worktree, message)?;
        self.push(worktree, false)
            .map_err(UsecaseError::after_commit)
    }

    /// commit の後に Sync する。commit が失敗したら Sync しない。
    /// Sync が失敗したときは、commit を作った後の失敗（`UsecaseError::AfterCommit`）を返す。
    pub fn commit_and_sync(&self, worktree: &str, message: &str) -> Result<(), UsecaseError> {
        self.commit(worktree, message)?;
        self.sync(worktree).map_err(UsecaseError::after_commit)
    }

    /// ブランチの commit を送る。送り先と送り方は worktree の Git の状態が決める。
    pub fn push(&self, worktree: &str, force_with_lease: bool) -> Result<(), UsecaseError> {
        let push = self
            .current_worktree_status(worktree)?
            .push_plan(force_with_lease)?;
        self.git.push(worktree, push)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    /// upstream の commit をブランチに取り込む。
    pub fn pull(&self, worktree: &str) -> Result<(), UsecaseError> {
        self.git.pull(worktree)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    /// Pull の後に Push する。Pull が失敗したら Push しない。
    pub fn sync(&self, worktree: &str) -> Result<(), UsecaseError> {
        self.pull(worktree)?;
        self.push(worktree, false)
    }

    /// remote の状態を取得する。
    pub fn fetch(&self, worktree: &str) -> Result<(), UsecaseError> {
        self.git.fetch(worktree)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    // ── 破棄（書き込み） ──

    /// 指定したファイルの、まだ stage していない変更を破棄する。破棄のしかたは worktree の Git の状態が決める。
    pub fn discard(&self, worktree: &str, paths: &[String]) -> Result<(), UsecaseError> {
        let plan: crate::domain::git::DiscardPlan = self
            .current_worktree_status(worktree)?
            .discard_plan(paths)?;
        self.git.restore_worktree_files(worktree, &plan.restore)?;
        self.git.trash_files(worktree, &plan.trash)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    // ── merge・rebase の続行と中止（書き込み） ──

    /// 進行中の merge か rebase を先へ進める。commit を作るかと、そのメッセージは、worktree の Git の状態が決める。
    pub fn continue_operation(&self, worktree: &str, message: &str) -> Result<(), UsecaseError> {
        let status = self.current_worktree_status(worktree)?;
        let operation = status.ensure_can_continue()?;
        let message = status.continue_message(message)?;
        self.git
            .continue_operation(worktree, operation, message.as_ref())?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    /// 進行中の merge か rebase を中止する。
    pub fn abort_operation(&self, worktree: &str) -> Result<(), UsecaseError> {
        let status = self.current_worktree_status(worktree)?;
        let operation = status.ensure_can_abort()?;
        self.git.abort_operation(worktree, operation)?;
        self.notify_repository_changed(worktree);
        Ok(())
    }

    // ── origin（読み取り） ──

    /// repository の origin の URL を返す。repository を開けない、または origin が無いときは `None`。
    pub fn origin_url(&self, repo_path: &str) -> Result<Option<String>, UsecaseError> {
        Ok(self.git.origin_url(repo_path)?)
    }

    // ── worktree（読み取り） ──

    pub fn get_main_repo_path(&self, any_path: &str) -> Result<String, UsecaseError> {
        Ok(self.git.main_repo_path(any_path)?)
    }

    pub fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, UsecaseError> {
        Ok(self.git.find_main_repo_path(path)?)
    }

    /// Workspaces に並べる worktree。隔離 worktree は含めず、main を先頭にブランチ名順で返す。
    pub fn list_working_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, UsecaseError> {
        let repository_root = self.git.main_repo_path(repo_path)?;
        let mut worktrees = self.git.list_worktrees(repo_path)?;
        worktrees
            .retain(|worktree| !is_isolated(&repository_root, &worktree.path, &worktree.branch));
        worktrees.sort_by(|left, right| {
            (!left.is_main, &left.branch).cmp(&(!right.is_main, &right.branch))
        });
        Ok(worktrees)
    }

    /// 走査で読めた worktree に、削除を受け付けた worktree を合わせる。
    /// 削除中は true。git の登録が先に消えた worktree は、削除が終わるまで末尾に残す。
    pub fn with_deleting_worktrees(
        &self,
        repository_root: &str,
        worktrees: Vec<Worktree>,
    ) -> Vec<(Worktree, bool)> {
        let mut rows = worktrees
            .into_iter()
            .map(|worktree| (worktree, false))
            .collect::<Vec<_>>();
        self.worktree_operations
            .for_each_deleting_worktree(|deleting| {
                if deleting.repository_root != repository_root {
                    return;
                }
                if let Some(row) = rows.iter_mut().find(|(worktree, _)| {
                    worktree.path == deleting.path
                        || deleting.branch.as_deref() == Some(worktree.branch.as_str())
                }) {
                    row.1 = true;
                    return;
                }
                let branch = deleting
                    .branch
                    .clone()
                    .unwrap_or_else(|| deleting.path.clone());
                if !is_isolated(repository_root, &deleting.path, &branch) {
                    rows.push((Worktree::being_deleted(&deleting.path, branch), true));
                }
            });
        rows
    }

    /// ローカルブランチと、その worktree があるか。隔離 worktree が使うブランチは含めない。
    /// ブランチに対応しない worktree（detached HEAD など）も worktree ありとして並べる。
    pub fn list_branches_with_worktree(
        &self,
        repo_path: &str,
    ) -> Result<Vec<(Branch, bool)>, UsecaseError> {
        let repository_root = self.git.main_repo_path(repo_path)?;
        let (isolated, working): (Vec<_>, Vec<_>) = self
            .git
            .list_worktrees(repo_path)?
            .into_iter()
            .partition(|worktree| is_isolated(&repository_root, &worktree.path, &worktree.branch));
        let working = self.with_deleting_worktrees(&repository_root, working);
        let mut branches = self
            .git
            .list_branches(repo_path)?
            .into_iter()
            .filter(|branch| {
                !branch.is_remote
                    && !isolated
                        .iter()
                        .any(|worktree| worktree.branch == branch.name)
            })
            .map(|branch| {
                let has_worktree = working
                    .iter()
                    .any(|(worktree, _)| worktree.branch == branch.name);
                (branch, has_worktree)
            })
            .collect::<Vec<_>>();
        for (worktree, _) in &working {
            if !branches
                .iter()
                .any(|(branch, _)| branch.name == worktree.branch)
            {
                branches.push((Branch::local(&worktree.branch), true));
            }
        }
        Ok(branches)
    }

    // ── worktree（書き込み） ──

    pub fn create_worktree(
        &self,
        repo_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<WorktreeEntryDto, UsecaseError> {
        let worktree_path = derive_worktree_path(repo_path, branch);
        let wt = self.git.create_worktree(
            repo_path,
            &worktree_path,
            branch,
            create_branch,
            base_branch,
        )?;
        // base 指定時は releash-base config を設定する（旧 gateway 内蔵処理を
        // usecase オーケストレーションへ引き上げ。set は旧実装どおり伝播する）。
        if let Some(base) = base_branch {
            self.git
                .set_branch_base_override(repo_path, branch, Some(base))?;
        }
        self.notify_repository_changed(repo_path);
        Ok(WorktreeEntryDto {
            name: wt.name,
            path: to_canonical_forward_slash(&wt.path),
            branch: wt.branch,
            is_main: wt.is_main,
            is_locked: wt.is_locked,
        })
    }

    /// worktree 削除の業務手順。
    ///
    pub async fn remove_worktree(
        &self,
        archives: &dyn WorktreeExecutionArchiver,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), UsecaseError> {
        let worktree_path = self.git.validate_removal(repo_path, worktree_path, force)?;
        let worktree_path = worktree_path.as_str();
        let mut deletion = archives
            .begin_worktree_deletion(worktree_path)
            .await
            .map_err(UsecaseError::from)?;
        self.git.validate_removal(repo_path, worktree_path, force)?;
        let repository_root = self.git.main_repo_path(worktree_path)?;
        let branch = match self.git.current(worktree_path) {
            Err(error @ GitError::Technical(_)) => return Err(error.into()),
            result => result.ok(),
        };
        archives
            .archive_worktree(worktree_path)
            .await
            .map_err(UsecaseError::from)?;
        // (1) 紐づく terminal surface を停止する。
        self.worktree_terminals.kill_by_worktree(worktree_path);

        let worktree_path = worktree_path.to_string();
        deletion.accept(
            crate::domain::git::worktree_operation::WorktreeDeletionTarget {
                repository_root,
                path: worktree_path.clone(),
                branch,
            },
        )?;
        self.notify_repository_changed(repo_path);
        let repository = self.clone();
        let repo_path = repo_path.to_string();
        let result = crate::common::operation_context::spawn_blocking(move || {
            let result = (|| {
                if let Some(branch) = repository.git.remove(&repo_path, &worktree_path, force)? {
                    repository
                        .git
                        .set_branch_base_override(&repo_path, &branch, None)?;
                }
                Ok::<_, UsecaseError>(())
            })();
            drop(deletion);
            repository.notify_repository_changed(&repo_path);
            result
        })
        .await
        .map_err(|error| {
            GitError::Technical(crate::domain::failure::TechnicalFailure::from(error))
        })?;
        result
    }

    // ── util / locator（読み取り） ──

    pub fn get_cwd(&self) -> Result<String, UsecaseError> {
        Ok(self.locator.cwd()?)
    }
}

fn is_isolated(repository_root: &str, worktree_path: &str, branch: &str) -> bool {
    crate::domain::workflow::WorktreeInventoryEntry::new(repository_root, worktree_path, branch)
        .matches_isolated_identity_rule()
}

#[cfg(test)]
#[path = "git_usecase_test.rs"]
mod git_usecase_tests;

#[cfg(feature = "test-support")]
impl GitUsecase {
    pub fn test_replace_worktree_repository(&mut self, git: Arc<dyn GitService>) {
        self.git = git;
    }
}
