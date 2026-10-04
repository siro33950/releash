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
    pub(crate) fn startup_worktree(
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

    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn worktree_operations(&self) -> Arc<WorktreeOperations> {
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
    pub(crate) fn with_deleting_worktrees(
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

#[cfg(test)]
mod repository_usecase_tests {
    use super::*;
    use crate::domain::repository::{RepositoryError, RepositoryStatusScan, Worktree};
    use parking_lot::Mutex;

    /// 委譲・順序・変換を検証するための記録付き手書き fake。
    /// 1 つの構造体で repository ドメインの全 trait を実装する。
    #[derive(Default)]
    struct FakeRepo {
        current_branch: String,
        fail_current_branch: bool,
        stop_current_branch: Option<crate::common::operation_context::OperationStopped>,
        worktrees: Vec<Worktree>,
        dirty: u32,
        branch_base: Option<String>,
        fail_create_worktree: bool,
        fail_remove_worktree: bool,
        fail_cleanup: bool,
        remove_started: tokio::sync::Notify,
        remove_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
        cleanup_started: tokio::sync::Notify,
        cleanup_continue: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
        fail_validate_removal: bool,
        operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
        fail_archive: bool,
        archive_continue: Option<tokio::sync::Notify>,
        archived_worktrees: Mutex<Vec<(String, usize)>>,
        created_branches: Mutex<Vec<String>>,
        removed_worktrees: Mutex<Vec<(String, bool)>>,
        /// `kill_by_worktree` 呼び出し時の (対象 path, その時点の removed 件数)。
        killed_worktree_terminals: Mutex<Vec<(String, usize)>>,
        /// `remove` が返す「削除した worktree のブランチ名」。
        removed_branch: Option<String>,
        set_branch_base_override_calls: Mutex<Vec<(String, Option<String>)>>,
        set_releash_base_calls: Mutex<Vec<Option<String>>>,
        prune_calls: Mutex<Vec<Vec<String>>>,
        fail_main_repo_path: bool,
        listed_worktree_paths: Mutex<Vec<String>>,
        branches: Vec<Branch>,
    }

    impl FakeRepo {
        async fn wait_for_deletion(&self, path: &str) {
            let identity =
                crate::adaptor::gateway::repository::worktree_operation::worktree_identity(path)
                    .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while self.operations.mutate(identity.to_str().unwrap()).is_err() {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("worktree deletion did not finish");
        }
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

    fn usecase(fake: Arc<FakeRepo>) -> RepositoryUsecase {
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

    /// 走査で 1 件も読めていないときに、削除中として残る worktree。
    fn deleting_rows(repository: &RepositoryUsecase) -> Vec<(Worktree, bool)> {
        repository.with_deleting_worktrees("/main", Vec::new())
    }

    fn wt(path: &str, branch: &str, is_main: bool) -> Worktree {
        Worktree {
            name: "n".to_string(),
            path: path.to_string(),
            branch: branch.to_string(),
            is_main,
            is_locked: false,
            is_merged: false,
        }
    }

    #[test]
    fn test_ブランチ作成を委譲する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone()).create_branch("/r", "feat").unwrap();
        assert_eq!(*fake.created_branches.lock(), vec!["feat".to_string()]);
    }

    #[test]
    fn test_worktree作成をdtoへ合成する() {
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        let entry = usecase(fake.clone())
            .create_worktree("/r", "feat/issues/1302", true, Some("main"))
            // When
            .unwrap();
        // Then
        assert_eq!(entry.branch, "feat/issues/1302");
        assert_eq!(entry.path, "/r-worktrees/feat-issues-1302");

        // base 指定時は usecase が releash-base を設定する（旧 gateway 内蔵処理の引き上げ）。
        assert_eq!(
            *fake.set_branch_base_override_calls.lock(),
            vec![("feat/issues/1302".to_string(), Some("main".to_string()))]
        );
    }

    #[test]
    fn test_worktree作成_base未指定ではbase設定しない() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .create_worktree("/r", "feat", true, None)
            .unwrap();
        assert!(fake.set_branch_base_override_calls.lock().is_empty());
    }

    #[tokio::test]
    async fn test_worktree削除_対応ブランチのbaseを後始末する() {
        // remove が返したブランチ名で releash-base を best-effort 削除する。
        let fake = Arc::new(FakeRepo {
            removed_branch: Some("feat".to_string()),
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/r", "/wt", false)
            .await
            .unwrap();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(
            *fake.set_branch_base_override_calls.lock(),
            vec![("feat".to_string(), None)]
        );
    }

    #[test]
    fn test_worktree一覧_pathとbranchとis_mainを写す() {
        // Given
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt("/wt-feat", "feat", false)],
            ..<FakeRepo as Default>::default()
        });
        // When
        let entries = usecase(fake).list_worktrees("/r").unwrap();
        // Then
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.path, "/wt-feat");
        assert_eq!(e.branch, "feat");
        assert!(!e.is_main);
    }

    #[test]
    fn test_linked_worktreeのreadには開いたrepo_pathを渡す() {
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt("/linked", "feature", false)],
            ..<FakeRepo as Default>::default()
        });
        let repository = usecase(fake.clone());

        repository.list_worktrees("/linked").unwrap();
        repository.list_working_worktrees("/linked").unwrap();
        repository.list_branches_with_worktree("/linked").unwrap();

        assert_eq!(*fake.listed_worktree_paths.lock(), vec!["/linked"; 3]);
    }

    #[test]
    fn test_repository_root解決に失敗したreadはqueryを呼ばずに失敗する() {
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt("/linked", "feature", false)],
            fail_main_repo_path: true,
            ..<FakeRepo as Default>::default()
        });
        let repository = usecase(fake.clone());

        assert!(repository.list_worktrees("/linked").is_err());
        assert!(repository.list_working_worktrees("/linked").is_err());
        assert!(repository.list_branches_with_worktree("/linked").is_err());

        assert!(fake.listed_worktree_paths.lock().is_empty());
    }

    #[test]
    fn test_作業worktree一覧_隔離worktreeを除きmainを先頭にブランチ名順で返す() {
        // Given
        let fake = Arc::new(FakeRepo {
            worktrees: vec![
                wt("/main-worktrees/feature-b", "feature-b", false),
                wt(
                    "/main-worktrees/.releash-isolated/node-a1",
                    "releash/isolated/node-a1",
                    false,
                ),
                wt("/main", "main", true),
                wt(
                    "/main-worktrees/other-a1",
                    "releash/isolated/other-a1",
                    false,
                ),
                wt("/main-worktrees/feature-a", "feature-a", false),
            ],
            ..<FakeRepo as Default>::default()
        });
        // When
        let worktrees = usecase(fake).list_working_worktrees("/main").unwrap();
        // Then
        assert_eq!(
            worktrees
                .iter()
                .map(|worktree| worktree.branch.as_str())
                .collect::<Vec<_>>(),
            vec![
                "main",
                "feature-a",
                "feature-b",
                "releash/isolated/other-a1"
            ]
        );
    }

    #[test]
    fn test_ブランチ状態_worktreeの有無を付け隔離worktreeのブランチを含めない() {
        // Given
        let fake = Arc::new(FakeRepo {
            branches: vec![
                Branch::local("feature"),
                Branch::local("main"),
                Branch::local("releash/isolated/retained-a1"),
                Branch::local("releash/isolated/hidden-a1"),
                Branch::remote("origin/main"),
            ],
            worktrees: vec![
                wt("/main", "main", true),
                wt(
                    "/main-worktrees/.releash-isolated/hidden-a1",
                    "releash/isolated/hidden-a1",
                    false,
                ),
                wt("/main-worktrees/detached", "detached-head", false),
            ],
            ..<FakeRepo as Default>::default()
        });
        // When
        let branches = usecase(fake).list_branches_with_worktree("/main").unwrap();
        // Then
        assert_eq!(
            branches,
            vec![
                (Branch::local("feature"), false),
                (Branch::local("main"), true),
                (Branch::local("releash/isolated/retained-a1"), false),
                (Branch::local("detached-head"), true),
            ]
        );
    }

    #[tokio::test]
    async fn test_削除中worktree_管理情報が消えても対象だけ残しguard終了で消す() {
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        let repository = usecase(fake.clone());
        let mut deletion = fake
            .operations
            .delete("/main-worktrees/feature")
            .await
            .unwrap();
        deletion
            .accept(
                crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                    repository_root: "/main".into(),
                    path: "/main-worktrees/feature".into(),
                    branch: Some("feature".into()),
                },
            )
            .unwrap();
        for path in [
            Some("/main-worktrees/feature"),
            Some("/alias/feature"),
            None,
        ] {
            let scanned = std::iter::once(wt("/main", "main", true))
                .chain(path.map(|path| wt(path, "feature", false)))
                .collect();
            // When
            let rows = repository.with_deleting_worktrees("/main", scanned);
            // Then
            assert_eq!(rows.len(), 2);
            assert!(!rows[0].1);
            assert!(rows[1].1);
            assert_eq!(rows[1].0.branch, "feature");
            assert_eq!(rows[1].0.path, path.unwrap_or("/main-worktrees/feature"));
        }
        drop(deletion);
        let rows = repository.with_deleting_worktrees("/main", vec![wt("/main", "main", true)]);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].1);
        assert!(fake.operations.mutate("/main-worktrees/feature").is_ok());
    }

    #[tokio::test]
    async fn test_削除中worktree_ブランチ名が不明でもパスごとに対象を残す() {
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        let repository = usecase(fake.clone());
        let mut deletions = Vec::new();
        for path in ["/main-worktrees/one", "/main-worktrees/two"] {
            let mut deletion = fake.operations.delete(path).await.unwrap();
            deletion
                .accept(
                    crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                        repository_root: "/main".into(),
                        path: path.into(),
                        branch: None,
                    },
                )
                .unwrap();
            deletions.push(deletion);
        }
        // When
        let rows = repository.with_deleting_worktrees(
            "/main",
            vec![
                wt("/main-worktrees/other", "unknown", false),
                wt("/main-worktrees/one", "feature", false),
            ],
        );
        // Then
        assert_eq!(rows.len(), 3);
        assert!(!rows[0].1);
        assert_eq!(rows[1].0.branch, "feature");
        assert!(rows[1].1);
        assert_eq!(rows[2].0.branch, "/main-worktrees/two");
        assert_eq!(rows[2].0.path, "/main-worktrees/two");
        assert!(rows[2].1);
        drop(deletions);
        assert!(deleting_rows(&repository).is_empty());
    }

    #[test]
    fn test_worktree一覧_完全な隔離命名の実体を非表示にする() {
        let fake = Arc::new(FakeRepo {
            worktrees: vec![
                wt(
                    "/main-worktrees/.releash-isolated/orphan-a1",
                    "releash/isolated/orphan-a1",
                    false,
                ),
                wt("/main-worktrees/feature", "feature", false),
            ],
            ..<FakeRepo as Default>::default()
        });

        let entries = usecase(fake).list_worktrees("/main").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch, "feature");
    }

    #[test]
    fn test_worktree一覧のpathはunc_prefixを保持して正規化する() {
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt(r"\\server\share\wt-feat", "feat", false)],
            ..<FakeRepo as Default>::default()
        });

        let entries = usecase(fake).list_worktrees("/r").unwrap();

        assert_eq!(entries[0].path, "//server/share/wt-feat");
    }

    #[test]
    fn test_worktree作成はunc_repo_pathから意味保存でpathを導出する() {
        let fake = Arc::new(<FakeRepo as Default>::default());

        let entry = usecase(fake)
            .create_worktree(r"\\server\share\repo", "feat/issues/1302", true, None)
            .unwrap();

        assert_eq!(entry.path, "//server/share/repo-worktrees/feat-issues-1302");
    }

    #[test]
    fn test_worktree作成エラーをusecaseエラーへ変換する() {
        let fake = Arc::new(FakeRepo {
            fail_create_worktree: true,
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake)
            .create_worktree("/r", "feat", true, None)
            .unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn test_gcを委譲する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .prune_stale_branch_bases("/r", &["a".to_string(), "b".to_string()])
            .unwrap();
        assert_eq!(
            *fake.prune_calls.lock(),
            vec![vec!["a".to_string(), "b".to_string()]]
        );
    }

    #[test]
    fn test_config設定系を委譲する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        let uc = usecase(fake.clone());
        uc.set_releash_base("/r", Some("dev")).unwrap();
        uc.set_branch_base_override("/r", "feat", Some("main"))
            .unwrap();
        assert_eq!(
            *fake.set_releash_base_calls.lock(),
            vec![Some("dev".to_string())]
        );
        assert_eq!(
            *fake.set_branch_base_override_calls.lock(),
            vec![("feat".to_string(), Some("main".to_string()))]
        );
    }

    #[test]
    fn test_起動worktree_一件だけのときpathと表示名を返す() {
        for count in [0, 1, 2] {
            let fake = Arc::new(FakeRepo {
                worktrees: (0..count)
                    .map(|index| Worktree {
                        name: format!("wt{index}"),
                        path: format!("/main/wt{index}"),
                        branch: format!("feature{index}"),
                        is_main: index == 0,
                        is_locked: false,
                        is_merged: false,
                    })
                    .collect(),
                ..Default::default()
            });
            let result = usecase(fake).startup_worktree().unwrap();
            if count == 1 {
                let worktree = result.unwrap();
                assert_eq!(worktree.path, "/main/wt0");
                assert_eq!(worktree.branch, "feature0");
                assert_eq!(worktree.repository_name, "main");
            } else {
                assert!(result.is_none());
            }
        }
    }

    #[tokio::test]
    async fn test_worktree削除_base後始末の失敗を返し削除中を解いて通知する() {
        let fake = Arc::new(FakeRepo {
            removed_branch: Some("feature".into()),
            fail_cleanup: true,
            ..Default::default()
        });
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = subscriptions.changes();
        let repository = usecase(fake.clone()).with_state_publisher(subscriptions);
        let error = repository
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "cleanup failed");
        assert!(deleting_rows(&repository).is_empty());
        assert!(fake.operations.mutate("/wt").is_ok());
        assert_eq!(
            changes.try_recv().unwrap(),
            crate::usecase::state_subscription::StateChangeSource::Repository(vec!["/repo".into()])
        );
        assert_eq!(
            changes.try_recv().unwrap(),
            crate::usecase::state_subscription::StateChangeSource::Repository(vec!["/repo".into()])
        );
    }

    #[tokio::test]
    async fn test_worktree削除を委譲する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/r", "/wt", false)
            .await
            .unwrap();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(
            *fake.removed_worktrees.lock(),
            vec![("/wt".to_string(), false)]
        );
    }

    #[tokio::test]
    async fn test_worktree削除_紐づくterminal_surfaceを先に停止する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/r", "/wt", false)
            .await
            .unwrap();
        fake.wait_for_deletion("/wt").await;
        // worktree 本体の削除（removed 0 件時点）より前に停止が呼ばれる。
        assert_eq!(
            *fake.killed_worktree_terminals.lock(),
            vec![("/wt".to_string(), 0)]
        );
        assert_eq!(
            *fake.removed_worktrees.lock(),
            vec![("/wt".to_string(), false)]
        );
    }

    #[tokio::test]
    async fn test_worktree削除_削除失敗を返しterminal停止は実行する() {
        let fake = Arc::new(FakeRepo {
            fail_remove_worktree: true,
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/r", "/wt", false)
            .await
            .unwrap_err();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(
            *fake.killed_worktree_terminals.lock(),
            vec![("/wt".to_string(), 0)]
        );
        // 削除に失敗した場合は releash-base の後始末を行わない。
        assert!(fake.set_branch_base_override_calls.lock().is_empty());
    }

    #[tokio::test]
    async fn test_worktree削除_archive失敗ではterminalとフォルダを削除しない() {
        // Given
        let fake = Arc::new(FakeRepo {
            fail_archive: true,
            ..Default::default()
        });
        // When
        let error = usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .unwrap_err();
        // Then
        assert_eq!(error.to_string(), "archive failed");
        assert!(fake.removed_worktrees.lock().is_empty());
        assert!(fake.killed_worktree_terminals.lock().is_empty());
    }

    #[tokio::test]
    async fn test_worktree削除_archiveがフォルダ削除に先行する() {
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        // When
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .unwrap();
        fake.wait_for_deletion("/wt").await;
        // Then
        assert_eq!(
            *fake.archived_worktrees.lock(),
            vec![("/wt".to_string(), 0)]
        );
        assert_eq!(fake.removed_worktrees.lock().len(), 1);
    }
    #[tokio::test]
    async fn test_worktree削除_所属不一致とlockedとdirtyではarchiveしない() {
        for (invalid, locked, dirty) in [(true, false, 0), (false, true, 0), (false, false, 1)] {
            // Given
            let mut worktree = wt("/wt", "feature", false);
            worktree.is_locked = locked;
            let fake = Arc::new(FakeRepo {
                fail_validate_removal: invalid,
                dirty,
                worktrees: vec![worktree],
                ..Default::default()
            });
            // When
            let result = usecase(fake.clone())
                .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                .await;
            // Then
            assert!(result.is_err());
            assert!(fake.archived_worktrees.lock().is_empty());
            assert!(fake.killed_worktree_terminals.lock().is_empty());
            assert!(fake.removed_worktrees.lock().is_empty());
        }
    }

    #[tokio::test]
    async fn test_worktree削除_実gitの拒否条件では実行木を変更しない() {
        use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
        use crate::test_support::git::{create_initial_commit, create_test_repo};
        for condition in ["wrong-repo", "locked", "dirty", "valid", "archive-failure"] {
            // Given
            let (repo_dir, repo) = create_test_repo();
            create_initial_commit(&repo);
            let worktrees = tempfile::tempdir().unwrap();
            let path = worktrees.path().join("feature");
            let worktree = repo.worktree("feature", &path, None).unwrap();
            let (other_dir, _other) = create_test_repo();
            if condition == "locked" {
                worktree.lock(None).unwrap();
            }
            if condition == "dirty" {
                std::fs::write(path.join("dirty"), "change").unwrap();
            }
            let fake = Arc::new(FakeRepo {
                fail_archive: condition == "archive-failure",
                ..Default::default()
            });
            let mut usecase = usecase(fake.clone());
            usecase.worktree = Arc::new(WorktreeGateway);
            let root = if condition == "wrong-repo" {
                other_dir.path()
            } else {
                repo_dir.path()
            };
            // When
            let result = usecase
                .remove_worktree(
                    fake.as_ref(),
                    root.to_str().unwrap(),
                    path.to_str().unwrap(),
                    false,
                )
                .await;
            // Then
            assert_eq!(result.is_ok(), condition == "valid");
            fake.wait_for_deletion(path.to_str().unwrap()).await;
            assert_eq!(
                fake.archived_worktrees.lock().len(),
                usize::from(matches!(condition, "valid" | "archive-failure"))
            );
            assert_eq!(path.exists(), condition != "valid");
            assert_eq!(repo.find_worktree("feature").is_ok(), condition != "valid");
        }
    }

    #[tokio::test]
    async fn test_worktree削除_先行変更の待機後に削除条件を再検証する() {
        use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
        use crate::test_support::git::{create_initial_commit, create_test_repo};

        for condition in ["dirty", "locked", "unregistered", "valid", "force-dirty"] {
            // Given
            let (repo_dir, repo) = create_test_repo();
            create_initial_commit(&repo);
            let worktrees = tempfile::tempdir().unwrap();
            let path = worktrees.path().canonicalize().unwrap().join("feature");
            let worktree = repo.worktree("feature", &path, None).unwrap();
            let root = repo_dir.path().to_str().unwrap();
            let path_str = path.to_str().unwrap();
            let fake = Arc::new(FakeRepo {
                current_branch: "main".into(),
                ..Default::default()
            });
            let mut repository = usecase(fake.clone());
            repository.worktree = Arc::new(WorktreeGateway);
            let mutation = fake.operations.mutate(path_str).unwrap();
            let force = condition == "force-dirty";
            let deletion = repository.remove_worktree(fake.as_ref(), root, path_str, force);
            tokio::pin!(deletion);

            // When
            assert!(futures_util::poll!(&mut deletion).is_pending());
            assert!(fake.archived_worktrees.lock().is_empty());
            assert!(fake.operations.mutate(path_str).is_err());
            match condition {
                "dirty" | "force-dirty" => {
                    std::fs::write(path.join("dirty"), "change").unwrap();
                }
                "locked" => worktree.lock(None).unwrap(),
                "unregistered" => {
                    std::fs::remove_dir_all(repo.path().join("worktrees/feature")).unwrap();
                }
                _ => {}
            }
            drop(mutation);
            let result = deletion.await;
            fake.wait_for_deletion(path_str).await;

            // Then
            let accepted = matches!(condition, "valid" | "force-dirty");
            assert_eq!(result.is_ok(), accepted, "{condition}");
            assert_eq!(fake.archived_worktrees.lock().len(), usize::from(accepted));
            assert_eq!(
                fake.killed_worktree_terminals.lock().len(),
                usize::from(accepted)
            );
            assert_eq!(path.exists(), !accepted);
            assert!(fake.operations.mutate(path_str).is_ok());
        }
    }

    #[tokio::test]
    async fn test_worktree削除_受理後も削除終了まで一覧と排他を保つ() {
        for fail in [false, true] {
            // Given
            let (release, blocked) = std::sync::mpsc::channel();
            let fake = Arc::new(FakeRepo {
                current_branch: "feature".into(),
                removed_branch: Some("feature".into()),
                fail_remove_worktree: fail,
                remove_continue: Some(Mutex::new(blocked)),
                ..Default::default()
            });
            let repository = usecase(fake.clone());
            // When
            let deletion = tokio::spawn({
                let repository = repository.clone();
                let fake = fake.clone();
                async move {
                    repository
                        .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                        .await
                }
            });
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fake.remove_started.notified(),
            )
            .await
            .unwrap();
            // Then
            assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
            assert!(fake.operations.mutate("/wt").is_err());
            assert!(fake.operations.mutate("/other").is_ok());
            assert!(repository.get_repository_status_scan("/wt").is_ok());
            assert!(repository
                .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                .await
                .is_err());
            let rows = deleting_rows(&repository);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].0.branch, "feature");
            assert_eq!(rows[0].0.path, "/wt");
            assert!(rows[0].1);
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.set_branch_base_override_calls.lock().is_empty());
            assert!(!deletion.is_finished());
            release.send(()).unwrap();
            assert_eq!(deletion.await.unwrap().is_err(), fail);
            fake.wait_for_deletion("/wt").await;
            assert!(deleting_rows(&repository).is_empty());
            assert_eq!(fake.removed_worktrees.lock().len(), usize::from(!fail));
            assert_eq!(
                fake.set_branch_base_override_calls.lock().len(),
                usize::from(!fail)
            );
        }
    }

    #[tokio::test]
    async fn test_worktree削除_base設定の後始末が終わるまで一覧と排他を保つ() {
        // Given
        let (release, blocked) = std::sync::mpsc::channel();
        let fake = Arc::new(FakeRepo {
            current_branch: "feature".into(),
            removed_branch: Some("feature".into()),
            cleanup_continue: Some(Mutex::new(blocked)),
            ..Default::default()
        });
        let repository = usecase(fake.clone());
        // When
        let deletion = tokio::spawn({
            let repository = repository.clone();
            let fake = fake.clone();
            async move {
                repository
                    .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
                    .await
            }
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fake.cleanup_started.notified(),
        )
        .await
        .unwrap();
        // Then
        assert_eq!(fake.removed_worktrees.lock().len(), 1);
        let rows = deleting_rows(&repository);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].1);
        assert!(fake.operations.mutate("/wt").is_err());
        assert!(repository.get_repository_status_scan("/wt").is_ok());
        assert!(!deletion.is_finished());
        release.send(()).unwrap();
        deletion.await.unwrap().unwrap();
        fake.wait_for_deletion("/wt").await;
        assert!(deleting_rows(&repository).is_empty());
    }

    #[tokio::test]
    async fn test_worktree削除_archive完了前には受理も背景削除も一覧追加もしない() {
        // Given
        let fake = Arc::new(FakeRepo {
            archive_continue: Some(tokio::sync::Notify::new()),
            ..Default::default()
        });
        let repository = usecase(fake.clone());
        let removal = repository.remove_worktree(fake.as_ref(), "/repo", "/wt", false);
        tokio::pin!(removal);
        // When
        assert!(futures_util::poll!(&mut removal).is_pending());
        // Then
        assert!(fake.removed_worktrees.lock().is_empty());
        assert!(fake.killed_worktree_terminals.lock().is_empty());
        assert!(deleting_rows(&repository).is_empty());
        fake.archive_continue.as_ref().unwrap().notify_one();
        removal.await.unwrap();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(fake.removed_worktrees.lock().len(), 1);
    }

    #[tokio::test]
    async fn test_worktree削除_ブランチ取得の停止を保持し後続操作へ進まない() {
        use crate::common::operation_context::OperationStopped;
        for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
            for force in [false, true] {
                // Given
                let fake = Arc::new(FakeRepo {
                    stop_current_branch: Some(stopped),
                    ..Default::default()
                });
                let publisher = crate::test_support::state_subscription::test_subscriptions();
                let mut changes = crate::test_support::state_subscription::changes(&publisher);
                let repository = usecase(fake.clone()).with_state_publisher(publisher);
                // When
                let error = repository
                    .remove_worktree(fake.as_ref(), "/repo", "/wt", force)
                    .await
                    .unwrap_err();
                // Then
                assert!(
                    matches!(error, UsecaseError::Repository(crate::domain::repository::RepositoryError::Technical(ref actual)) if *actual == stopped.into())
                );
                assert!(fake.archived_worktrees.lock().is_empty());
                assert!(fake.killed_worktree_terminals.lock().is_empty());
                assert!(fake.removed_worktrees.lock().is_empty());
                assert!(fake.set_branch_base_override_calls.lock().is_empty());
                assert!(changes.try_recv().is_err());
                assert!(deleting_rows(&repository).is_empty());
                assert!(fake.operations.mutate("/wt").is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_worktree削除_ブランチ名を取得できなくてもarchive後に受理する() {
        for force in [false, true] {
            // Given
            let (release, blocked) = std::sync::mpsc::channel();
            let fake = Arc::new(FakeRepo {
                fail_current_branch: true,
                remove_continue: Some(Mutex::new(blocked)),
                ..Default::default()
            });
            let repository = usecase(fake.clone());
            // When
            let deletion = tokio::spawn({
                let repository = repository.clone();
                let fake = fake.clone();
                async move {
                    repository
                        .remove_worktree(fake.as_ref(), "/repo", "/wt", force)
                        .await
                }
            });
            fake.remove_started.notified().await;
            // Then
            assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
            assert_eq!(
                *fake.killed_worktree_terminals.lock(),
                vec![("/wt".into(), 0)]
            );
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.operations.mutate("/wt").is_err());
            let rows = deleting_rows(&repository);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].0.branch, "/wt");
            assert_eq!(rows[0].0.path, "/wt");
            assert!(rows[0].1);
            assert!(!deletion.is_finished());
            release.send(()).unwrap();
            deletion.await.unwrap().unwrap();
            fake.wait_for_deletion("/wt").await;
            assert_eq!(*fake.removed_worktrees.lock(), vec![("/wt".into(), force)]);
            assert!(deleting_rows(&repository).is_empty());
        }
    }

    #[tokio::test]
    async fn test_worktree削除_リポジトリ識別情報を取得できないときarchive前に拒否する() {
        // Given
        let fake = Arc::new(FakeRepo {
            fail_main_repo_path: true,
            ..Default::default()
        });
        let repository = usecase(fake.clone());
        // When
        assert!(repository
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .is_err());
        // Then
        assert!(fake.archived_worktrees.lock().is_empty());
        assert!(fake.removed_worktrees.lock().is_empty());
        assert!(fake.operations.mutate("/wt").is_ok());
    }
    #[tokio::test]
    async fn test_repository更新_操作成功後だけ購読へ通知する() {
        use crate::usecase::state_subscription::StateChangeSource;
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        let publisher = crate::test_support::state_subscription::test_subscriptions();
        let mut changes = crate::test_support::state_subscription::changes(&publisher);
        let uc = usecase(fake.clone()).with_state_publisher(publisher.clone());
        // When / Then
        uc.create_worktree("/repo", "feature", true, None).unwrap();
        assert_eq!(
            changes.try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        uc.set_releash_base("/repo", Some("main")).unwrap();
        assert_eq!(
            changes.try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        uc.set_branch_base_override("/repo", "feature", Some("main"))
            .unwrap();
        assert_eq!(
            changes.try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        uc.remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .unwrap();
        assert_eq!(
            changes.try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        fake.wait_for_deletion("/wt").await;
        assert_eq!(
            changes.try_recv().unwrap(),
            StateChangeSource::Repository(vec!["/repo".into()])
        );
        let failed = Arc::new(FakeRepo {
            fail_create_worktree: true,
            fail_validate_removal: true,
            ..Default::default()
        });
        let failed_uc = usecase(failed.clone()).with_state_publisher(publisher);
        assert!(failed_uc
            .create_worktree("/repo", "feature", true, None)
            .is_err());
        assert!(failed_uc
            .remove_worktree(failed.as_ref(), "/repo", "/wt", false)
            .await
            .is_err());
        assert!(changes.try_recv().is_err());
    }
}

fn is_isolated(repository_root: &str, worktree_path: &str, branch: &str) -> bool {
    crate::domain::workflow::WorktreeInventoryEntry::new(repository_root, worktree_path, branch)
        .matches_isolated_identity_rule()
}
