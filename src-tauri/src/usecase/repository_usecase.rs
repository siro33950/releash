//! repository ドメインの Command 側ユースケース（業務手順）と、読み取りの集約入口。
//!
//! controller / watcher / workflow はこの Usecase だけを入口とする。
//! 書き込み・複数集約のオーケストレーションに加え、Entity をそのまま返す読み取りも
//! Repository へ委譲してここから提供する。表示・転送向けの read model（DTO）の生成だけは
//! 読み取りクエリサービス（協力者）へ委譲する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::path::to_canonical_forward_slash;
use crate::domain::repository::{
    worktree_path as derive_worktree_path, Branch, BranchRepository, GitConfigRepository,
    RepoLocator, RepositoryError, RepositoryStatusScan, StatusRepository, WorktreeRepository,
    WorktreeTerminalGateway,
};

use super::repository_dto::{BranchCardDto, WorktreeEntryDto};
use super::repository_error::UsecaseError;
use super::repository_query_service::RepositoryQueryService;

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
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionPublisher>,
    branch: Arc<dyn BranchRepository>,
    status: Arc<dyn StatusRepository>,
    worktree: Arc<dyn WorktreeRepository>,
    git_config: Arc<dyn GitConfigRepository>,
    locator: Arc<dyn RepoLocator>,
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    query: RepositoryQueryService,
}

impl RepositoryUsecase {
    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn worktree_operations(&self) -> Arc<super::worktree_operation::WorktreeOperations> {
        self.query.worktree_operations.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: Arc<dyn BranchRepository>,
        status: Arc<dyn StatusRepository>,
        worktree: Arc<dyn WorktreeRepository>,
        git_config: Arc<dyn GitConfigRepository>,
        locator: Arc<dyn RepoLocator>,
        worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
        query: RepositoryQueryService,
    ) -> Self {
        Self {
            state_publisher: None,
            branch,
            status,
            worktree,
            git_config,
            locator,
            worktree_terminals,
            query,
        }
    }

    fn notify_repository_changed(&self, path: &str) {
        if let Some(publisher) = &self.state_publisher {
            publisher.invalidate(
                crate::domain::state_subscription::StateChangeSource::Repository(vec![path.into()]),
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

    /// ブランチ削除の業務手順。
    ///
    /// (1) 既定ブランチ・(2) メインワークツリーでチェックアウト中のブランチは
    /// 削除を拒否し、(3) 紐づく worktree を先に削除（git の機構的制約による順序）、
    /// (4) ブランチ本体を削除、(5) releash-base config を後始末する。
    /// 複数集約（branch / worktree / git_config）をまたぐオーケストレーションは
    /// usecase の責務であり、gateway は単一集約のプリミティブに分解する。
    pub async fn delete_branch(
        &self,
        archives: &dyn WorktreeExecutionArchiver,
        repo_path: &str,
        branch_name: &str,
        force: bool,
    ) -> Result<(), UsecaseError> {
        self.validate_branch_deletion(repo_path, branch_name)?;

        // (3) 紐づく worktree を先に削除（checkout 中ブランチは削除不可のため）。
        //     先に壊れた linked worktree を prune してリカバリーする（旧実装の
        //     削除前リカバリーと同順）。削除した worktree の releash-base 後始末は
        //     ブランチ単位で (5) がまとめて行うため、ここでは戻り値を無視する。
        let targets = self
            .worktree
            .list(repo_path)?
            .into_iter()
            .filter(|wt| !wt.is_main && wt.branch == branch_name)
            .map(|wt| self.worktree.validate_removal(repo_path, &wt.path, force))
            .collect::<Result<Vec<_>, _>>()?;
        let mut deletions = Vec::new();
        for path in &targets {
            deletions.push(
                archives
                    .begin_worktree_deletion(path)
                    .await
                    .map_err(UsecaseError::from)?,
            );
        }
        let invalid_paths = self.worktree.invalid_worktree_paths(repo_path)?;
        for path in &invalid_paths {
            deletions.push(
                archives
                    .begin_worktree_deletion(path)
                    .await
                    .map_err(UsecaseError::from)?,
            );
        }
        self.validate_branch_deletion(repo_path, branch_name)?;
        for path in &targets {
            self.worktree.validate_removal(repo_path, path, force)?;
        }
        for path in &invalid_paths {
            archives
                .archive_worktree(path)
                .await
                .map_err(UsecaseError::from)?;
        }
        for path in &targets {
            archives
                .archive_worktree(path)
                .await
                .map_err(UsecaseError::from)?;
        }
        self.worktree.prune_invalid(repo_path)?;
        for path in &targets {
            self.worktree_terminals.kill_by_worktree(path);
            self.worktree.remove(repo_path, path, force)?;
        }

        // (4) ブランチ本体を削除。
        self.branch.delete(repo_path, branch_name)?;

        // (5) releash-base config を後始末する（best-effort）。ブランチ本体削除
        //     という主目的の成功後に config 掃除が失敗しても全体を失敗にしない
        //     （旧実装と等価。リトライ時の branch not found 化を防ぐ）。
        let _ = self
            .git_config
            .set_branch_base_override(repo_path, branch_name, None);

        Ok(())
    }

    fn validate_branch_deletion(
        &self,
        repo_path: &str,
        branch_name: &str,
    ) -> Result<(), UsecaseError> {
        // (1) 既定ブランチの削除を拒否（既定が検出できない場合は拒否しない）。
        if let Ok(default) = self.branch.default(repo_path) {
            if default == branch_name {
                return Err(UsecaseError::Rule(
                    "cannot delete the default branch".to_string(),
                ));
            }
        }

        // (2) メインワークツリーでチェックアウト中のブランチの削除を拒否。
        if self.branch.current(repo_path)? == branch_name {
            return Err(UsecaseError::Rule(
                "cannot delete the branch currently checked out in the main worktree".to_string(),
            ));
        }

        Ok(())
    }

    // ── status（読み取り） ──

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

    /// worktree 一覧の read model を組み立てる。worktree 識別情報（worktree 集約）に
    /// `dirty_count`（status 集約）と `base_branch`（git_config 集約）を合成する複数集約の
    /// オーケストレーション。各 worktree 自身のパスで解決し、停止以外の失敗時は 0 / None に倒す
    /// （旧 gateway の一覧構築と等価）。
    pub fn list_worktrees(&self, repo_path: &str) -> Result<Vec<WorktreeEntryDto>, UsecaseError> {
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        let worktrees = self.worktree.list(repo_path)?;
        let mut entries = Vec::with_capacity(worktrees.len());
        for wt in worktrees {
            let dirty_count = match self.worktree.dirty_count(&wt.path) {
                Err(error @ RepositoryError::Technical(_)) => return Err(error.into()),
                result => result.unwrap_or(0),
            };
            let base_branch = match self.git_config.get_branch_base(&wt.path, &wt.branch) {
                Err(error @ RepositoryError::Technical(_)) => return Err(error.into()),
                result => result.unwrap_or(None),
            };
            entries.push(WorktreeEntryDto {
                name: wt.name,
                path: to_canonical_forward_slash(&wt.path),
                branch: wt.branch,
                is_main: wt.is_main,
                is_locked: wt.is_locked,
                dirty_count,
                base_branch,
            });
        }
        self.query
            .classify_worktree_entries(&repository_root, &mut entries);
        Ok(entries)
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
        // 新規作成直後は dirty_count = 0、base_branch は指定値（旧 gateway 戻り値と等価）。
        self.notify_repository_changed(repo_path);
        Ok(WorktreeEntryDto {
            name: wt.name,
            path: to_canonical_forward_slash(&wt.path),
            branch: wt.branch,
            is_main: wt.is_main,
            is_locked: wt.is_locked,
            dirty_count: 0,
            base_branch: base_branch.map(|s| s.to_string()),
        })
    }

    /// worktree 削除の業務手順。
    ///
    /// 検証・Archive・terminal 停止後に受理し、削除と base 設定の後始末は
    /// 排他を保持した背景タスクで行う。受理後の成否は保持・通知しない。
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
        tokio::task::spawn_blocking(move || {
            let _deletion = deletion;
            if let Ok(Some(branch)) = repository
                .worktree
                .remove(&repo_path, &worktree_path, force)
            {
                let _ = repository
                    .git_config
                    .set_branch_base_override(&repo_path, &branch, None);
            }
        });
        Ok(())
    }

    pub(crate) fn include_deleting_worktrees(
        &self,
        repository_root: &str,
        cards: &mut Vec<BranchCardDto>,
    ) {
        self.query
            .include_deleting_worktrees(repository_root, cards);
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

    /// ブランチカード read model を副作用なしで取得する。
    ///
    /// snapshot scanner は watcher invalidate から実行される read model 更新経路なので、
    /// config GC のような書き込みを伴う [`list_branches_with_status`] ではなくこちらを使う。
    #[cfg(test)]
    pub fn list_branches_with_status_read_only(
        &self,
        repo_path: &str,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        self.query
            .list_branches_with_status(repo_path, &repository_root)
    }

    pub fn list_branches_with_status_for_scan(
        &self,
        repo_path: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        let repository_root = self.worktree.main_repo_path(repo_path)?;
        self.query.list_branches_with_status_for_scan(
            repo_path,
            &repository_root,
            current_dirty_count,
        )
    }
}

#[cfg(test)]
mod repository_usecase_tests {
    use super::*;
    use crate::domain::repository::{RepositoryError, RepositoryStatusScan, Worktree};
    use crate::usecase::repository_query_service::BranchCardQuery;
    use parking_lot::Mutex;

    /// 委譲・順序・変換を検証するための記録付き手書き fake。
    /// 1 つの構造体で repository ドメインの全 trait を実装する。
    #[derive(Default)]
    struct FakeRepo {
        default_branch: Option<String>,
        current_branch: String,
        fail_current_branch: bool,
        stop_current_branch: Option<crate::common::operation_context::OperationStopped>,
        worktrees: Vec<Worktree>,
        invalid_worktrees: Vec<String>,
        dirty: u32,
        stop_dirty: Option<crate::common::operation_context::OperationStopped>,
        stop_base: Option<crate::common::operation_context::OperationStopped>,
        detail_calls: Mutex<Vec<&'static str>>,
        branch_base: Option<String>,
        fail_create_worktree: bool,
        fail_remove_worktree: bool,
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
        deleted_branches: Mutex<Vec<String>>,
        removed_worktrees: Mutex<Vec<(String, bool)>>,
        /// `kill_by_worktree` 呼び出し時の (対象 path, その時点の removed 件数)。
        killed_worktree_terminals: Mutex<Vec<(String, usize)>>,
        /// `remove` が返す「削除した worktree のブランチ名」。
        removed_branch: Option<String>,
        prune_invalid_calls: Mutex<u32>,
        set_branch_base_override_calls: Mutex<Vec<(String, Option<String>)>>,
        set_releash_base_calls: Mutex<Vec<Option<String>>>,
        prune_calls: Mutex<Vec<Vec<String>>>,
        fail_main_repo_path: bool,
        listed_worktree_paths: Mutex<Vec<String>>,
        branch_card_paths: Mutex<Vec<String>>,
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
            Ok(Vec::new())
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
        fn default(&self, _repo_path: &str) -> Result<String, RepositoryError> {
            self.default_branch
                .clone()
                .ok_or_else(|| RepositoryError::rule("no default branch found"))
        }
        fn create(&self, _repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
            self.created_branches.lock().push(branch_name.to_string());
            Ok(())
        }
        fn delete(&self, _repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
            self.deleted_branches.lock().push(branch_name.to_string());
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
        fn main_repo_path(&self, _any_path: &str) -> Result<String, RepositoryError> {
            if self.fail_main_repo_path {
                return Err(RepositoryError::External(
                    "main repo path is unavailable".to_string(),
                ));
            }
            Ok("/main".to_string())
        }
        fn dirty_count(&self, _worktree_path: &str) -> Result<u32, RepositoryError> {
            self.detail_calls.lock().push("dirty");
            if let Some(stopped) = self.stop_dirty {
                return Err(stopped.into());
            }
            Ok(self.dirty)
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
        fn invalid_worktree_paths(&self, _repo_path: &str) -> Result<Vec<String>, RepositoryError> {
            Ok(self.invalid_worktrees.clone())
        }
        fn prune_invalid(&self, _repo_path: &str) -> Result<(), RepositoryError> {
            *self.prune_invalid_calls.lock() += 1;
            Ok(())
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
            self.detail_calls.lock().push("base");
            if let Some(stopped) = self.stop_base {
                return Err(stopped.into());
            }
            Ok(self.branch_base.clone())
        }
        fn set_branch_base_override(
            &self,
            _repo_path: &str,
            branch_name: &str,
            base: Option<&str>,
        ) -> Result<(), RepositoryError> {
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

    impl BranchCardQuery for FakeRepo {
        fn list_branch_cards(
            &self,
            repo_path: &str,
        ) -> Result<Vec<BranchCardDto>, RepositoryError> {
            self.branch_card_paths.lock().push(repo_path.to_string());
            Ok(Vec::new())
        }

        fn list_branch_cards_for_scan(
            &self,
            repo_path: &str,
            _current_dirty_count: usize,
        ) -> Result<Vec<BranchCardDto>, RepositoryError> {
            self.branch_card_paths.lock().push(repo_path.to_string());
            Ok(Vec::new())
        }
    }

    #[test]
    fn test_worktree一覧_詳細取得の停止を既定値に変えず後続を呼ばない() {
        use crate::common::operation_context::OperationStopped;
        // Given
        for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
            for stop_dirty in [true, false] {
                let fake = Arc::new(FakeRepo {
                    stop_dirty: stop_dirty.then_some(stopped),
                    stop_base: (!stop_dirty).then_some(stopped),
                    worktrees: vec![
                        Worktree {
                            name: "main".into(),
                            path: "/main".into(),
                            branch: "main".into(),
                            is_main: true,
                            is_locked: false
                        };
                        2
                    ],
                    ..Default::default()
                });
                // When
                let error = usecase(fake.clone()).list_worktrees("/main").unwrap_err();
                // Then
                assert!(
                    matches!(error, UsecaseError::Repository(crate::domain::repository::RepositoryError::Technical(ref actual)) if *actual == stopped.into())
                );
                assert_eq!(
                    *fake.detail_calls.lock(),
                    if stop_dirty {
                        vec!["dirty"]
                    } else {
                        vec!["dirty", "base"]
                    }
                );
            }
        }
    }

    fn usecase(fake: Arc<FakeRepo>) -> RepositoryUsecase {
        let query = RepositoryQueryService::new(fake.clone(), fake.operations.clone());
        RepositoryUsecase::new(
            fake.clone(),
            fake.clone(),
            fake.clone(),
            fake.clone(),
            fake.clone(),
            fake.clone(),
            query,
        )
    }

    fn wt(path: &str, branch: &str, is_main: bool) -> Worktree {
        Worktree {
            name: "n".to_string(),
            path: path.to_string(),
            branch: branch.to_string(),
            is_main,
            is_locked: false,
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
        let fake = Arc::new(<FakeRepo as Default>::default());
        let entry = usecase(fake.clone())
            .create_worktree("/r", "feat/issues/1302", true, Some("main"))
            .unwrap();
        assert_eq!(entry.branch, "feat/issues/1302");
        assert_eq!(entry.path, "/r-worktrees/feat-issues-1302");
        // 新規作成直後は dirty_count = 0、base_branch は指定値。
        assert_eq!(entry.dirty_count, 0);
        assert_eq!(entry.base_branch, Some("main".to_string()));
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
    fn test_worktree一覧をdtoへ合成する() {
        // slim worktree（識別情報）に dirty_count（status）・base_branch（git_config）を
        // usecase が合成して read model を組み立てる。
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt("/wt-feat", "feat", false)],
            dirty: 3,
            branch_base: Some("develop".to_string()),
            ..<FakeRepo as Default>::default()
        });
        let entries = usecase(fake).list_worktrees("/r").unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.path, "/wt-feat");
        assert_eq!(e.branch, "feat");
        assert!(!e.is_main);
        assert_eq!(e.dirty_count, 3);
        assert_eq!(e.base_branch, Some("develop".to_string()));
    }

    #[test]
    fn test_linked_worktreeのreadには開いたrepo_pathを渡す() {
        let fake = Arc::new(FakeRepo {
            worktrees: vec![wt("/linked", "feature", false)],
            ..<FakeRepo as Default>::default()
        });
        let repository = usecase(fake.clone());

        repository.list_worktrees("/linked").unwrap();
        repository
            .list_branches_with_status_for_scan("/linked", 1)
            .unwrap();

        assert_eq!(*fake.listed_worktree_paths.lock(), vec!["/linked"]);
        assert_eq!(*fake.branch_card_paths.lock(), vec!["/linked"]);
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
        assert!(repository
            .list_branches_with_status_read_only("/linked")
            .is_err());
        assert!(repository
            .list_branches_with_status_for_scan("/linked", 1)
            .is_err());

        assert!(fake.listed_worktree_paths.lock().is_empty());
        assert!(fake.branch_card_paths.lock().is_empty());
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

    #[tokio::test]
    async fn test_ブランチ削除_既定ブランチ拒否() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake.clone())
            .delete_branch(fake.as_ref(), "/r", "main", false)
            .await
            .unwrap_err();
        assert!(matches!(err, UsecaseError::Rule(_)));
        assert!(err.to_string().contains("default branch"));
        assert!(fake.deleted_branches.lock().is_empty());
    }

    #[tokio::test]
    async fn test_ブランチ削除_チェックアウト中拒否() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            current_branch: "feat".to_string(),
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake.clone())
            .delete_branch(fake.as_ref(), "/r", "feat", false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("currently checked out"));
        assert!(fake.deleted_branches.lock().is_empty());
    }

    #[tokio::test]
    async fn test_ブランチ削除_既定未検出でも削除可() {
        // default() が Err（既定未検出）でも拒否せず削除する
        let fake = Arc::new(FakeRepo {
            current_branch: "main".to_string(),
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .delete_branch(fake.as_ref(), "/r", "feat", false)
            .await
            .unwrap();
        assert_eq!(*fake.deleted_branches.lock(), vec!["feat".to_string()]);
    }

    #[tokio::test]
    async fn test_ブランチ削除_紐づくworktreeを先に削除し後始末する() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            current_branch: "main".to_string(),
            worktrees: vec![wt("/main", "main", true), wt("/wt-feat", "feat", false)],
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .delete_branch(fake.as_ref(), "/r", "feat", true)
            .await
            .unwrap();
        // 削除前に壊れた worktree の prune（リカバリー）を実行する
        assert_eq!(*fake.prune_invalid_calls.lock(), 1);
        // 紐づく非メイン worktree を force 伝播で削除
        assert_eq!(
            *fake.removed_worktrees.lock(),
            vec![("/wt-feat".to_string(), true)]
        );
        // ブランチ本体を削除
        assert_eq!(*fake.deleted_branches.lock(), vec!["feat".to_string()]);
        // releash-base を後始末（None で削除）
        assert_eq!(
            *fake.set_branch_base_override_calls.lock(),
            vec![("feat".to_string(), None)]
        );
    }

    #[tokio::test]
    async fn test_ブランチ削除_invalid_worktreeもarchive失敗ならpruneしない() {
        let fake = Arc::new(FakeRepo {
            current_branch: "main".into(),
            invalid_worktrees: vec!["/gone".into()],
            fail_archive: true,
            ..Default::default()
        });
        assert!(usecase(fake.clone())
            .delete_branch(fake.as_ref(), "/r", "feat", false)
            .await
            .is_err());
        assert_eq!(*fake.prune_invalid_calls.lock(), 0);
        assert!(fake.deleted_branches.lock().is_empty());
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
    async fn test_worktree削除_受理後の削除失敗は返さずterminal停止は実行する() {
        let fake = Arc::new(FakeRepo {
            fail_remove_worktree: true,
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .remove_worktree(fake.as_ref(), "/r", "/wt", false)
            .await
            .unwrap();
        fake.wait_for_deletion("/wt").await;
        assert_eq!(
            *fake.killed_worktree_terminals.lock(),
            vec![("/wt".to_string(), 0)]
        );
        // 削除に失敗した場合は releash-base の後始末を行わない。
        assert!(fake.set_branch_base_override_calls.lock().is_empty());
    }

    #[test]
    fn test_read_onlyブランチカード一覧取得ではgcしない() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .list_branches_with_status_read_only("/r")
            .unwrap();
        assert!(fake.prune_calls.lock().is_empty());
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
    async fn test_linked_worktreeブランチ削除_archiveがremoveに先行し失敗時はどちらも削除しない() {
        for fail_archive in [false, true] {
            // Given
            let fake = Arc::new(FakeRepo {
                current_branch: "main".into(),
                worktrees: vec![wt("/wt", "feature", false)],
                fail_archive,
                ..Default::default()
            });
            // When
            let result = usecase(fake.clone())
                .delete_branch(fake.as_ref(), "/repo", "feature", false)
                .await;
            // Then
            assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
            assert_eq!(result.is_err(), fail_archive);
            assert_eq!(
                fake.removed_worktrees.lock().len(),
                usize::from(!fail_archive)
            );
            assert_eq!(
                fake.deleted_branches.lock().len(),
                usize::from(!fail_archive)
            );
            assert_eq!(*fake.prune_invalid_calls.lock(), u32::from(!fail_archive));
        }
    }

    #[tokio::test]
    async fn test_linked_worktreeブランチ削除_dirtyとlockedでは付随pruneもarchiveもしない() {
        for (locked, dirty) in [(true, 0), (false, 1)] {
            // Given
            let mut worktree = wt("/wt", "feature", false);
            worktree.is_locked = locked;
            let fake = Arc::new(FakeRepo {
                current_branch: "main".into(),
                worktrees: vec![worktree],
                invalid_worktrees: vec!["/invalid".into()],
                dirty,
                ..Default::default()
            });
            // When
            assert!(usecase(fake.clone())
                .delete_branch(fake.as_ref(), "/repo", "feature", false)
                .await
                .is_err());
            // Then
            assert!(fake.archived_worktrees.lock().is_empty());
            assert_eq!(*fake.prune_invalid_calls.lock(), 0);
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.deleted_branches.lock().is_empty());
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

        for delete_branch in [false, true] {
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
                let deletion = async {
                    if delete_branch {
                        repository
                            .delete_branch(fake.as_ref(), root, "feature", force)
                            .await
                    } else {
                        repository
                            .remove_worktree(fake.as_ref(), root, path_str, force)
                            .await
                    }
                };
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
                assert_eq!(result.is_ok(), accepted, "{delete_branch}: {condition}");
                assert_eq!(fake.archived_worktrees.lock().len(), usize::from(accepted));
                assert_eq!(
                    fake.killed_worktree_terminals.lock().len(),
                    usize::from(accepted)
                );
                assert_eq!(
                    fake.deleted_branches.lock().len(),
                    usize::from(delete_branch && accepted)
                );
                assert_eq!(path.exists(), !accepted);
                assert!(fake.operations.mutate(path_str).is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_ブランチ削除_prune対象の待機中にdirtyになった場合もarchiveしない() {
        use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
        use crate::test_support::git::{create_initial_commit, create_test_repo};

        // Given
        let (repo_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let worktrees = tempfile::tempdir().unwrap();
        let path = worktrees.path().canonicalize().unwrap().join("feature");
        repo.worktree("feature", &path, None).unwrap();
        let invalid_path = worktrees.path().canonicalize().unwrap().join("invalid");
        repo.worktree("invalid", &invalid_path, None).unwrap();
        std::fs::remove_dir_all(&invalid_path).unwrap();
        let fake = Arc::new(FakeRepo {
            current_branch: "main".into(),
            ..Default::default()
        });
        let mut repository = usecase(fake.clone());
        repository.worktree = Arc::new(WorktreeGateway);
        let mutation = fake
            .operations
            .mutate(invalid_path.to_str().unwrap())
            .unwrap();
        let deletion = repository.delete_branch(
            fake.as_ref(),
            repo_dir.path().to_str().unwrap(),
            "feature",
            false,
        );
        tokio::pin!(deletion);

        // When
        assert!(futures_util::poll!(&mut deletion).is_pending());
        std::fs::write(path.join("dirty"), "change").unwrap();
        drop(mutation);
        let result = deletion.await;

        // Then
        assert!(result.is_err());
        assert!(fake.archived_worktrees.lock().is_empty());
        assert!(fake.killed_worktree_terminals.lock().is_empty());
        assert!(fake.deleted_branches.lock().is_empty());
        assert!(repo.find_worktree("invalid").is_ok());
        assert!(path.exists());
        assert!(fake.operations.mutate(path.to_str().unwrap()).is_ok());
        assert!(fake
            .operations
            .mutate(invalid_path.to_str().unwrap())
            .is_ok());
    }

    #[tokio::test]
    async fn test_ブランチ削除_待機中にmainでcheckoutされたらarchiveしない() {
        use crate::adaptor::gateway::repository::{
            branch::BranchGateway, worktree::WorktreeGateway,
        };
        use crate::test_support::git::{create_initial_commit, create_test_repo};

        // Given
        let (repo_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let worktrees = tempfile::tempdir().unwrap();
        let path = worktrees.path().canonicalize().unwrap().join("feature");
        repo.worktree("feature", &path, None).unwrap();
        let fake = Arc::new(<FakeRepo as Default>::default());
        let mut repository = usecase(fake.clone());
        repository.worktree = Arc::new(WorktreeGateway);
        repository.branch = Arc::new(BranchGateway);
        let mutation = fake.operations.mutate(path.to_str().unwrap()).unwrap();
        let deletion = repository.delete_branch(
            fake.as_ref(),
            repo_dir.path().to_str().unwrap(),
            "feature",
            false,
        );
        tokio::pin!(deletion);

        // When
        assert!(futures_util::poll!(&mut deletion).is_pending());
        git2::Repository::open(&path)
            .unwrap()
            .set_head_detached(repo.head().unwrap().target().unwrap())
            .unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        drop(mutation);
        let error = deletion.await.unwrap_err();

        // Then
        assert!(error.to_string().contains("currently checked out"));
        assert!(fake.archived_worktrees.lock().is_empty());
        assert!(fake.killed_worktree_terminals.lock().is_empty());
        assert!(repo.find_worktree("feature").is_ok());
        assert!(repo.find_branch("feature", git2::BranchType::Local).is_ok());
        assert!(path.exists());
        assert!(fake.operations.mutate(path.to_str().unwrap()).is_ok());
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
            let repository = Arc::new(usecase(fake.clone()));
            let state = crate::usecase::repository_state::RepositoryStateService::new(crate::usecase::work_queue::WorkQueueUsecase::new(crate::usecase::work_queue_test_runtime::runtime()),
                Arc::new(crate::adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(repository.clone())),
                Arc::new(crate::adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                    repository.clone(), Arc::new(crate::adaptor::controller::wiring::build_code_usecase())
                )),
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateNotifier),
                Arc::new(crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher),
                Arc::new(crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime),
                Arc::new(crate::usecase::repository_state::runtime::tests_support::IdentityWorktreePathNormalizer),
            );
            // When
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                repository.remove_worktree(fake.as_ref(), "/repo", "/wt", false),
            )
            .await
            .unwrap()
            .unwrap();
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
            let mut cards = Vec::new();
            repository.include_deleting_worktrees("/main", &mut cards);
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].name, "feature");
            assert_eq!(cards[0].worktree_path.as_deref(), Some("/wt"));
            assert!(cards[0].is_deleting);
            let snapshot = state.list_branches_with_status_snapshot("/repo").unwrap();
            assert!(snapshot.branches[0].is_deleting);
            assert_eq!(snapshot.worktree_display_groups.working_areas.len(), 1);
            assert!(snapshot.worktree_display_groups.working_areas[0].is_deleting);
            let wire: crate::adaptor::protocol::client::BranchCardDto =
                cards.remove(0).try_into().unwrap();
            assert_eq!(wire.is_deleting, Some(true));
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.set_branch_base_override_calls.lock().is_empty());
            release.send(()).unwrap();
            fake.wait_for_deletion("/wt").await;
            let mut cards = Vec::new();
            repository.include_deleting_worktrees("/main", &mut cards);
            assert!(cards.is_empty());
            assert!(state
                .list_branches_with_status_snapshot("/repo")
                .unwrap()
                .worktree_display_groups
                .working_areas
                .is_empty());
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
        repository
            .remove_worktree(fake.as_ref(), "/repo", "/wt", false)
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fake.cleanup_started.notified(),
        )
        .await
        .unwrap();
        // Then
        assert_eq!(fake.removed_worktrees.lock().len(), 1);
        let mut cards = Vec::new();
        repository.include_deleting_worktrees("/main", &mut cards);
        assert_eq!(cards.len(), 1);
        assert!(cards[0].is_deleting);
        assert!(fake.operations.mutate("/wt").is_err());
        assert!(repository.get_repository_status_scan("/wt").is_ok());
        release.send(()).unwrap();
        fake.wait_for_deletion("/wt").await;
        let mut cards = Vec::new();
        repository.include_deleting_worktrees("/main", &mut cards);
        assert!(cards.is_empty());
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
        let mut cards = Vec::new();
        repository.include_deleting_worktrees("/main", &mut cards);
        assert!(cards.is_empty());
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
                let publisher =
                    crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
                let mut changes = publisher.subscribe_changes();
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
                let mut cards = Vec::new();
                repository.include_deleting_worktrees("/main", &mut cards);
                assert!(cards.is_empty());
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
            repository
                .remove_worktree(fake.as_ref(), "/repo", "/wt", force)
                .await
                .unwrap();
            // Then
            assert_eq!(*fake.archived_worktrees.lock(), vec![("/wt".into(), 0)]);
            assert_eq!(
                *fake.killed_worktree_terminals.lock(),
                vec![("/wt".into(), 0)]
            );
            assert!(fake.removed_worktrees.lock().is_empty());
            assert!(fake.operations.mutate("/wt").is_err());
            let mut cards = Vec::new();
            repository.include_deleting_worktrees("/main", &mut cards);
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].name, "/wt");
            assert_eq!(cards[0].worktree_path.as_deref(), Some("/wt"));
            assert!(cards[0].is_deleting);
            release.send(()).unwrap();
            fake.wait_for_deletion("/wt").await;
            assert_eq!(*fake.removed_worktrees.lock(), vec![("/wt".into(), force)]);
            let mut cards = Vec::new();
            repository.include_deleting_worktrees("/main", &mut cards);
            assert!(cards.is_empty());
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
        use crate::domain::state_subscription::StateChangeSource;
        // Given
        let fake = Arc::new(<FakeRepo as Default>::default());
        let publisher = crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
        let mut changes = publisher.subscribe_changes();
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
