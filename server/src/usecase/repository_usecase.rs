//! repository ドメインの Command 側ユースケース（業務手順）と、読み取りの集約入口。
//!
//! controller / watcher / workflow はこの Usecase だけを入口とする。
//! 書き込み・複数集約のオーケストレーションに加え、Entity をそのまま返す読み取りも
//! Repository へ委譲してここから提供する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::path::to_canonical_forward_slash;
use crate::domain::repository::{
    worktree_path as derive_worktree_path, Branch, BranchRepository, GitConfigRepository,
    RepoLocator, RepositoryError, RepositoryStatusScan, StatusRepository, Worktree,
    WorktreeRepository, WorktreeTerminalGateway,
};

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

#[derive(Clone)]
pub struct RepositoryUsecase {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    branch: Arc<dyn BranchRepository>,
    status: Arc<dyn StatusRepository>,
    worktree: Arc<dyn WorktreeRepository>,
    git_config: Arc<dyn GitConfigRepository>,
    locator: Arc<dyn RepoLocator>,
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    worktree_operations: Arc<WorktreeOperations>,
}

impl RepositoryUsecase {
    pub fn startup_worktree(
        &self,
    ) -> Result<Option<crate::usecase::repository_dto::StartupWorktree>, UsecaseError> {
        let Some(root) = self.find_main_repo_path(&self.get_cwd()?)? else {
            return Ok(None);
        };
        let mut worktrees = self.list_worktrees(&root)?;
        if worktrees.len() != 1 {
            return Ok(None);
        }
        let worktree = worktrees.remove(0);
        let repository_name = std::path::Path::new(&root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(root);
        Ok(Some(crate::usecase::repository_dto::StartupWorktree {
            path: worktree.path,
            branch: worktree.branch,
            repository_name,
        }))
    }

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

    pub fn new(
        branch: Arc<dyn BranchRepository>,
        status: Arc<dyn StatusRepository>,
        worktree: Arc<dyn WorktreeRepository>,
        git_config: Arc<dyn GitConfigRepository>,
        locator: Arc<dyn RepoLocator>,
        worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
        worktree_operations: Arc<WorktreeOperations>,
    ) -> Self {
        Self {
            state_publisher: None,
            branch,
            status,
            worktree,
            git_config,
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

    pub fn branch_tracking(
        &self,
        repo_path: &str,
        branch: &str,
    ) -> Result<Option<crate::domain::repository::BranchTracking>, UsecaseError> {
        Ok(self.branch.tracking(repo_path, branch)?)
    }

    pub fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, UsecaseError> {
        Ok(self.branch.list(repo_path)?)
    }

    pub fn get_current_branch(&self, repo_path: &str) -> Result<String, UsecaseError> {
        Ok(self.branch.current(repo_path)?)
    }

    // ── branch（書き込み） ──

    pub fn create_branch(&self, repo_path: &str, branch_name: &str) -> Result<(), UsecaseError> {
        self.branch.create(repo_path, branch_name)?;
        Ok(())
    }

    pub fn get_repository_status_scan(
        &self,
        repo_path: &str,
    ) -> Result<RepositoryStatusScan, UsecaseError> {
        Ok(self.status.status_scan(repo_path)?)
    }

    // ── worktree（読み取り） ──

    pub fn get_main_repo_path(&self, any_path: &str) -> Result<String, UsecaseError> {
        Ok(self.worktree.main_repo_path(any_path)?)
    }

    pub fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, UsecaseError> {
        Ok(self.worktree.find_main_repo_path(path)?)
    }

    pub fn list_worktrees(&self, repo_path: &str) -> Result<Vec<WorktreeEntryDto>, UsecaseError> {
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        let worktrees = self.worktree.list(repo_path)?;
        let mut entries = Vec::with_capacity(worktrees.len());
        for wt in worktrees {
            entries.push(WorktreeEntryDto {
                name: wt.name,
                path: to_canonical_forward_slash(&wt.path),
                branch: wt.branch,
                is_main: wt.is_main,
                is_locked: wt.is_locked,
            });
        }
        entries.retain(|entry| !is_isolated(&repository_root, &entry.path, &entry.branch));
        Ok(entries)
    }

    /// Workspaces に並べる worktree。隔離 worktree は含めず、main を先頭にブランチ名順で返す。
    pub fn list_working_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, UsecaseError> {
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        let mut worktrees = self.worktree.list(repo_path)?;
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
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        let (isolated, working): (Vec<_>, Vec<_>) = self
            .worktree
            .list(repo_path)?
            .into_iter()
            .partition(|worktree| is_isolated(&repository_root, &worktree.path, &worktree.branch));
        let working = self.with_deleting_worktrees(&repository_root, working);
        let mut branches = self
            .branch
            .list(repo_path)?
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
        let wt = self.worktree.create(
            repo_path,
            &worktree_path,
            branch,
            create_branch,
            base_branch,
        )?;
        // base 指定時は releash-base config を設定する（旧 gateway 内蔵処理を
        // usecase オーケストレーションへ引き上げ。set は旧実装どおり伝播する）。
        if let Some(base) = base_branch {
            self.git_config
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
        let worktree_path = self
            .worktree
            .validate_removal(repo_path, worktree_path, force)?;
        let worktree_path = worktree_path.as_str();
        let mut deletion = archives
            .begin_worktree_deletion(worktree_path)
            .await
            .map_err(UsecaseError::from)?;
        self.worktree
            .validate_removal(repo_path, worktree_path, force)?;
        let repository_root = self.worktree.main_repo_path(worktree_path)?;
        let branch = match self.branch.current(worktree_path) {
            Err(error @ RepositoryError::Technical(_)) => return Err(error.into()),
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
            crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
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
                if let Some(branch) =
                    repository
                        .worktree
                        .remove(&repo_path, &worktree_path, force)?
                {
                    repository
                        .git_config
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
            RepositoryError::Technical(crate::domain::failure::TechnicalFailure::from(error))
        })?;
        result
    }

    // ── git_config（読み取り） ──

    pub fn get_releash_base(&self, repo_path: &str) -> Result<Option<String>, UsecaseError> {
        Ok(self.git_config.get_releash_base(repo_path)?)
    }

    pub fn get_branch_base(
        &self,
        repo_path: &str,
        branch_name: &str,
    ) -> Result<Option<String>, UsecaseError> {
        Ok(self.git_config.get_branch_base(repo_path, branch_name)?)
    }

    // ── git_config（書き込み） ──

    pub fn set_releash_base(
        &self,
        repo_path: &str,
        base: Option<&str>,
    ) -> Result<(), UsecaseError> {
        self.git_config.set_releash_base(repo_path, base)?;
        self.notify_repository_changed(repo_path);
        Ok(())
    }

    pub fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), UsecaseError> {
        self.git_config
            .set_branch_base_override(repo_path, branch_name, base)?;
        self.notify_repository_changed(repo_path);
        Ok(())
    }

    /// ブランチ一覧取得後の GC（現存しないブランチの `releash-base` 掃除）。
    /// 読み取りクエリの副作用ではなく、明示的な Command として実行する。
    pub fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), UsecaseError> {
        self.git_config
            .prune_stale_branch_bases(repo_path, existing_branches)?;
        Ok(())
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
#[path = "repository_usecase_test.rs"]
mod repository_usecase_tests;

#[cfg(feature = "test-support")]
impl RepositoryUsecase {
    pub fn test_replace_worktree_repository(&mut self, worktree: Arc<dyn WorktreeRepository>) {
        self.worktree = worktree;
    }
}
