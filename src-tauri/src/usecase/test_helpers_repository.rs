use super::*;
use crate::domain::repository::{RepositoryError, RepositoryStatusScan, Worktree};
use parking_lot::Mutex;

/// 委譲・順序・変換を検証するための記録付き手書き fake。
/// 1 つの構造体で repository ドメインの全 trait を実装する。
#[derive(Default)]
pub struct FakeRepo {
    pub current_branch: String,
    pub fail_current_branch: bool,
    pub stop_current_branch: Option<crate::common::operation_context::OperationStopped>,
    pub worktrees: Vec<Worktree>,
    pub dirty: u32,
    pub branch_base: Option<String>,
    pub fail_create_worktree: bool,
    pub fail_remove_worktree: bool,
    pub fail_cleanup: bool,
    pub remove_started: tokio::sync::Notify,
    pub remove_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    pub cleanup_started: tokio::sync::Notify,
    pub cleanup_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
    pub fail_validate_removal: bool,
    pub operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    pub fail_archive: bool,
    pub archive_continue: Option<tokio::sync::Notify>,
    pub archived_worktrees: Mutex<Vec<(String, usize)>>,
    pub created_branches: Mutex<Vec<String>>,
    pub removed_worktrees: Mutex<Vec<(String, bool)>>,
    /// `kill_by_worktree` 呼び出し時の (対象 path, その時点の removed 件数)。
    pub killed_worktree_terminals: Mutex<Vec<(String, usize)>>,
    /// `remove` が返す「削除した worktree のブランチ名」。
    pub removed_branch: Option<String>,
    pub set_branch_base_override_calls: Mutex<Vec<(String, Option<String>)>>,
    pub set_releash_base_calls: Mutex<Vec<Option<String>>>,
    pub prune_calls: Mutex<Vec<Vec<String>>>,
    pub fail_main_repo_path: bool,
    pub listed_worktree_paths: Mutex<Vec<String>>,
    pub branches: Vec<Branch>,
}

#[async_trait::async_trait]
impl WorktreeExecutionArchiver for FakeRepo {
    async fn begin_worktree_deletion(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeDeletionGuard,
        crate::domain::workflow::WorkflowError,
    > {
        self.operations.delete(path).await.map_err(|error| {
            crate::domain::workflow::WorkflowError::invalid_state(error.to_string())
        })
    }
    async fn archive_worktree(
        &self,
        path: &str,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        self.archived_worktrees
            .lock()
            .push((path.to_string(), self.removed_worktrees.lock().len()));
        if self.fail_archive {
            return Err(crate::domain::workflow::WorkflowError::external(
                "archive failed",
            ));
        }
        if let Some(ready) = &self.archive_continue {
            ready.notified().await;
        }
        Ok(())
    }
}

impl BranchRepository for FakeRepo {
    fn list(&self, _repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
        Ok(self.branches.clone())
    }
    fn current(&self, _repo_path: &str) -> Result<String, RepositoryError> {
        if let Some(stopped) = self.stop_current_branch {
            return Err(stopped.into());
        }
        if self.fail_current_branch {
            return Err(RepositoryError::External("branch unavailable".into()));
        }
        Ok(self.current_branch.clone())
    }
    fn create(&self, _repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        self.created_branches.lock().push(branch_name.to_string());
        Ok(())
    }
}

impl StatusRepository for FakeRepo {
    fn status_scan(&self, _repo_path: &str) -> Result<RepositoryStatusScan, RepositoryError> {
        Ok(RepositoryStatusScan {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
        })
    }
}

impl WorktreeRepository for FakeRepo {
    fn find_main_repo_path(&self, path: &str) -> Result<Option<String>, RepositoryError> {
        self.main_repo_path(path).map(Some)
    }
    fn main_repo_path(&self, _any_path: &str) -> Result<String, RepositoryError> {
        if self.fail_main_repo_path {
            return Err(RepositoryError::External(
                "main repo path is unavailable".to_string(),
            ));
        }
        Ok("/main".to_string())
    }
    fn list(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
        self.listed_worktree_paths
            .lock()
            .push(repo_path.to_string());
        Ok(self.worktrees.clone())
    }
    fn create(
        &self,
        _repo_path: &str,
        worktree_path: &str,
        branch: &str,
        _create_branch: bool,
        _base_branch: Option<&str>,
    ) -> Result<Worktree, RepositoryError> {
        if self.fail_create_worktree {
            return Err(RepositoryError::External("boom".to_string()));
        }
        Ok(Worktree {
            name: "wt".to_string(),
            path: worktree_path.to_string(),
            branch: branch.to_string(),
            is_main: false,
            is_locked: false,
            is_merged: false,
        })
    }
    fn validate_removal(
        &self,
        _: &str,
        path: &str,
        force: bool,
    ) -> Result<String, RepositoryError> {
        if self.fail_validate_removal {
            return Err(RepositoryError::rule("worktree not found"));
        }
        let worktree = self
            .worktrees
            .iter()
            .find(|wt| wt.path == path)
            .cloned()
            .unwrap_or_else(|| wt(path, "feat", false));
        worktree.authorize_removal(force, self.dirty)?;
        Ok(path.to_string())
    }
    fn remove(
        &self,
        _repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<Option<String>, RepositoryError> {
        self.remove_started.notify_one();
        if let Some(receiver) = &self.remove_continue {
            receiver
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        if self.fail_remove_worktree {
            return Err(RepositoryError::External("remove failed".to_string()));
        }
        self.removed_worktrees
            .lock()
            .push((worktree_path.to_string(), force));
        Ok(self.removed_branch.clone())
    }
}

impl GitConfigRepository for FakeRepo {
    fn get_releash_base(&self, _repo_path: &str) -> Result<Option<String>, RepositoryError> {
        Ok(None)
    }
    fn set_releash_base(
        &self,
        _repo_path: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError> {
        self.set_releash_base_calls
            .lock()
            .push(base.map(|s| s.to_string()));
        Ok(())
    }
    fn get_branch_base(
        &self,
        _repo_path: &str,
        _branch_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self.branch_base.clone())
    }
    fn set_branch_base_override(
        &self,
        _repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError> {
        if self.fail_cleanup {
            return Err(RepositoryError::External("cleanup failed".into()));
        }
        self.cleanup_started.notify_one();
        if let Some(receiver) = &self.cleanup_continue {
            receiver
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.set_branch_base_override_calls
            .lock()
            .push((branch_name.to_string(), base.map(|s| s.to_string())));
        Ok(())
    }
    fn prune_stale_branch_bases(
        &self,
        _repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryError> {
        self.prune_calls.lock().push(existing_branches.to_vec());
        Ok(())
    }
    fn resolve_current_base_branch(
        &self,
        _path_hint: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self.branch_base.clone())
    }
    fn resolve_base_commit_oid(
        &self,
        _path_hint: &str,
        _base_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(None)
    }
}

impl RepoLocator for FakeRepo {
    fn cwd(&self) -> Result<String, RepositoryError> {
        Ok("/cwd".to_string())
    }
}

impl WorktreeTerminalGateway for FakeRepo {
    fn kill_by_worktree(&self, worktree_path: &str) {
        let removed_so_far = self.removed_worktrees.lock().len();
        self.killed_worktree_terminals
            .lock()
            .push((worktree_path.to_string(), removed_so_far));
    }
}

pub fn usecase(fake: Arc<FakeRepo>) -> RepositoryUsecase {
    RepositoryUsecase::new(
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.clone(),
        fake.operations.clone(),
    )
}

pub fn wt(path: &str, branch: &str, is_main: bool) -> Worktree {
    Worktree {
        name: "n".to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        is_main,
        is_locked: false,
        is_merged: false,
    }
}

impl FakeRepo {
    pub async fn wait_for_deletion(&self, path: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while self.operations.mutate(path).is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("worktree deletion did not finish");
    }
}
#[cfg(test)]
pub fn deleting_rows(repository: &RepositoryUsecase) -> Vec<(Worktree, bool)> {
    repository.with_deleting_worktrees("/main", Vec::new())
}
