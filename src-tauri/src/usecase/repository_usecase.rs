//! repository ドメインの Command 側ユースケース（業務手順）と、読み取りの集約入口。
//!
//! controller / ws_server / watcher / workflow はこの Usecase だけを入口とする。
//! 書き込み・複数集約のオーケストレーションに加え、Entity をそのまま返す読み取りも
//! Repository へ委譲してここから提供する。表示・転送向けの read model（DTO）の生成だけは
//! 読み取りクエリサービス（協力者）へ委譲する。
//! ドメイン抽象（trait）のみに依存し、具体的な外部リソース実装は知らない。

use std::sync::Arc;

use crate::domain::repository::{
    Branch, BranchRepository, Commit, FileStatus, GitConfigRepository, LogRepository, RepoLocator,
    RepositoryStatusScan, StatusRepository, WorktreeRepository,
};

use super::repository_dto::{BranchCardDto, WorktreeEntryDto};
use super::repository_error::UsecaseError;
use super::repository_query_service::RepositoryQueryService;

#[derive(Clone)]
pub struct RepositoryUsecase {
    branch: Arc<dyn BranchRepository>,
    log: Arc<dyn LogRepository>,
    status: Arc<dyn StatusRepository>,
    worktree: Arc<dyn WorktreeRepository>,
    git_config: Arc<dyn GitConfigRepository>,
    locator: Arc<dyn RepoLocator>,
    query: RepositoryQueryService,
}

impl RepositoryUsecase {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        branch: Arc<dyn BranchRepository>,
        log: Arc<dyn LogRepository>,
        status: Arc<dyn StatusRepository>,
        worktree: Arc<dyn WorktreeRepository>,
        git_config: Arc<dyn GitConfigRepository>,
        locator: Arc<dyn RepoLocator>,
        query: RepositoryQueryService,
    ) -> Self {
        Self {
            branch,
            log,
            status,
            worktree,
            git_config,
            locator,
            query,
        }
    }

    // ── branch（読み取り） ──

    pub fn list_branches(&self, repo_path: &str) -> Result<Vec<Branch>, UsecaseError> {
        Ok(self.branch.list(repo_path)?)
    }

    pub fn get_current_branch(&self, repo_path: &str) -> Result<String, UsecaseError> {
        Ok(self.branch.current(repo_path)?)
    }

    pub fn get_default_branch(&self, repo_path: &str) -> Result<String, UsecaseError> {
        Ok(self.branch.default(repo_path)?)
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
    pub fn delete_branch(
        &self,
        repo_path: &str,
        branch_name: &str,
        force: bool,
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

        // (3) 紐づく worktree を先に削除（checkout 中ブランチは削除不可のため）。
        //     先に壊れた linked worktree を prune してリカバリーする（旧実装の
        //     削除前リカバリーと同順）。削除した worktree の releash-base 後始末は
        //     ブランチ単位で (5) がまとめて行うため、ここでは戻り値を無視する。
        self.worktree.prune_invalid(repo_path)?;
        for wt in self.worktree.list(repo_path)? {
            if !wt.is_main && wt.branch == branch_name {
                self.worktree.remove(repo_path, &wt.path, force)?;
            }
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

    // ── commit・log（読み取り） ──

    pub fn get_git_log(
        &self,
        repo_path: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Commit>, UsecaseError> {
        Ok(self.log.log(repo_path, limit)?)
    }

    // ── status（読み取り） ──

    pub fn get_git_status_include_ignored(
        &self,
        repo_path: &str,
    ) -> Result<Vec<FileStatus>, UsecaseError> {
        Ok(self.status.status_with_options(repo_path, true)?)
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

    /// worktree 一覧の read model を組み立てる。worktree 識別情報（worktree 集約）に
    /// `dirty_count`（status 集約）と `base_branch`（git_config 集約）を合成する複数集約の
    /// オーケストレーション。各 worktree 自身のパスで解決し、失敗時は 0 / None に倒す
    /// （旧 gateway の一覧構築と等価）。
    pub fn list_worktrees(&self, repo_path: &str) -> Result<Vec<WorktreeEntryDto>, UsecaseError> {
        let worktrees = self.worktree.list(repo_path)?;
        let mut entries = Vec::with_capacity(worktrees.len());
        for wt in worktrees {
            let dirty_count = self.worktree.dirty_count(&wt.path).unwrap_or(0);
            let base_branch = self
                .git_config
                .get_branch_base(&wt.path, &wt.branch)
                .unwrap_or(None);
            entries.push(WorktreeEntryDto {
                name: wt.name,
                path: wt.path,
                branch: wt.branch,
                is_main: wt.is_main,
                is_locked: wt.is_locked,
                dirty_count,
                base_branch,
            });
        }
        Ok(entries)
    }

    // ── worktree（書き込み） ──

    pub fn create_worktree(
        &self,
        repo_path: &str,
        worktree_path: &str,
        branch: &str,
        create_branch: bool,
        base_branch: Option<&str>,
    ) -> Result<WorktreeEntryDto, UsecaseError> {
        let wt =
            self.worktree
                .create(repo_path, worktree_path, branch, create_branch, base_branch)?;
        // base 指定時は releash-base config を設定する（旧 gateway 内蔵処理を
        // usecase オーケストレーションへ引き上げ。set は旧実装どおり伝播する）。
        if let Some(base) = base_branch {
            self.git_config
                .set_branch_base_override(repo_path, branch, Some(base))?;
        }
        // 新規作成直後は dirty_count = 0、base_branch は指定値（旧 gateway 戻り値と等価）。
        Ok(WorktreeEntryDto {
            name: wt.name,
            path: wt.path,
            branch: wt.branch,
            is_main: wt.is_main,
            is_locked: wt.is_locked,
            dirty_count: 0,
            base_branch: base_branch.map(|s| s.to_string()),
        })
    }

    pub fn remove_worktree(
        &self,
        repo_path: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), UsecaseError> {
        // gateway は worktree を削除し、指していたブランチ名を返す。対応する
        // releash-base config の後始末は usecase が best-effort で行う（旧 gateway
        // 内蔵の Ok|Err 握りつぶしと等価）。
        let removed_branch = self.worktree.remove(repo_path, worktree_path, force)?;
        if let Some(branch) = removed_branch {
            let _ = self
                .git_config
                .set_branch_base_override(repo_path, &branch, None);
        }
        Ok(())
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

    pub fn get_repo_git_dir(&self, file_path: &str) -> Result<String, UsecaseError> {
        Ok(self.locator.git_dir(file_path)?)
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
        self.query.list_branches_with_status(repo_path)
    }

    pub fn list_branches_with_status_for_scan(
        &self,
        repo_path: &str,
        current_dirty_count: usize,
    ) -> Result<Vec<BranchCardDto>, UsecaseError> {
        self.query
            .list_branches_with_status_for_scan(repo_path, current_dirty_count)
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
        worktrees: Vec<Worktree>,
        dirty: u32,
        branch_base: Option<String>,
        fail_create_worktree: bool,
        created_branches: Mutex<Vec<String>>,
        deleted_branches: Mutex<Vec<String>>,
        removed_worktrees: Mutex<Vec<(String, bool)>>,
        /// `remove` が返す「削除した worktree のブランチ名」。
        removed_branch: Option<String>,
        prune_invalid_calls: Mutex<u32>,
        set_branch_base_override_calls: Mutex<Vec<(String, Option<String>)>>,
        set_releash_base_calls: Mutex<Vec<Option<String>>>,
        prune_calls: Mutex<Vec<Vec<String>>>,
    }

    impl BranchRepository for FakeRepo {
        fn list(&self, _repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
            Ok(Vec::new())
        }
        fn current(&self, _repo_path: &str) -> Result<String, RepositoryError> {
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

    impl LogRepository for FakeRepo {
        fn log(
            &self,
            _repo_path: &str,
            _limit: Option<usize>,
        ) -> Result<Vec<Commit>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    impl StatusRepository for FakeRepo {
        fn status_with_options(
            &self,
            _repo_path: &str,
            include_ignored: bool,
        ) -> Result<Vec<FileStatus>, RepositoryError> {
            let _ = include_ignored;
            Ok(Vec::new())
        }
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
            Ok("/main".to_string())
        }
        fn dirty_count(&self, _worktree_path: &str) -> Result<u32, RepositoryError> {
            Ok(self.dirty)
        }
        fn list(&self, _repo_path: &str) -> Result<Vec<Worktree>, RepositoryError> {
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
        fn remove(
            &self,
            _repo_path: &str,
            worktree_path: &str,
            force: bool,
        ) -> Result<Option<String>, RepositoryError> {
            self.removed_worktrees
                .lock()
                .push((worktree_path.to_string(), force));
            Ok(self.removed_branch.clone())
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
            Ok(self.branch_base.clone())
        }
        fn set_branch_base_override(
            &self,
            _repo_path: &str,
            branch_name: &str,
            base: Option<&str>,
        ) -> Result<(), RepositoryError> {
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
        fn resolve_effective_base_branch(
            &self,
            _repo_path: &str,
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
        fn git_dir(&self, _file_path: &str) -> Result<String, RepositoryError> {
            Ok("/cwd/.git".to_string())
        }
    }

    impl BranchCardQuery for FakeRepo {
        fn list_branch_cards(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<BranchCardDto>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    fn usecase(fake: Arc<FakeRepo>) -> RepositoryUsecase {
        let query = RepositoryQueryService::new(fake.clone());
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
            .create_worktree("/r", "/wt", "feat", true, Some("main"))
            .unwrap();
        assert_eq!(entry.branch, "feat");
        assert_eq!(entry.path, "/wt");
        // 新規作成直後は dirty_count = 0、base_branch は指定値。
        assert_eq!(entry.dirty_count, 0);
        assert_eq!(entry.base_branch, Some("main".to_string()));
        // base 指定時は usecase が releash-base を設定する（旧 gateway 内蔵処理の引き上げ）。
        assert_eq!(
            *fake.set_branch_base_override_calls.lock(),
            vec![("feat".to_string(), Some("main".to_string()))]
        );
    }

    #[test]
    fn test_worktree作成_base未指定ではbase設定しない() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .create_worktree("/r", "/wt", "feat", true, None)
            .unwrap();
        assert!(fake.set_branch_base_override_calls.lock().is_empty());
    }

    #[test]
    fn test_worktree削除_対応ブランチのbaseを後始末する() {
        // remove が返したブランチ名で releash-base を best-effort 削除する。
        let fake = Arc::new(FakeRepo {
            removed_branch: Some("feat".to_string()),
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .remove_worktree("/r", "/wt", false)
            .unwrap();
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
    fn test_worktree作成エラーをusecaseエラーへ変換する() {
        let fake = Arc::new(FakeRepo {
            fail_create_worktree: true,
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake)
            .create_worktree("/r", "/wt", "feat", true, None)
            .unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn test_ブランチ削除_既定ブランチ拒否() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake.clone())
            .delete_branch("/r", "main", false)
            .unwrap_err();
        assert!(matches!(err, UsecaseError::Rule(_)));
        assert!(err.to_string().contains("default branch"));
        assert!(fake.deleted_branches.lock().is_empty());
    }

    #[test]
    fn test_ブランチ削除_チェックアウト中拒否() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            current_branch: "feat".to_string(),
            ..<FakeRepo as Default>::default()
        });
        let err = usecase(fake.clone())
            .delete_branch("/r", "feat", false)
            .unwrap_err();
        assert!(err.to_string().contains("currently checked out"));
        assert!(fake.deleted_branches.lock().is_empty());
    }

    #[test]
    fn test_ブランチ削除_既定未検出でも削除可() {
        // default() が Err（既定未検出）でも拒否せず削除する
        let fake = Arc::new(FakeRepo {
            current_branch: "main".to_string(),
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .delete_branch("/r", "feat", false)
            .unwrap();
        assert_eq!(*fake.deleted_branches.lock(), vec!["feat".to_string()]);
    }

    #[test]
    fn test_ブランチ削除_紐づくworktreeを先に削除し後始末する() {
        let fake = Arc::new(FakeRepo {
            default_branch: Some("main".to_string()),
            current_branch: "main".to_string(),
            worktrees: vec![wt("/main", "main", true), wt("/wt-feat", "feat", false)],
            ..<FakeRepo as Default>::default()
        });
        usecase(fake.clone())
            .delete_branch("/r", "feat", true)
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
    fn test_worktree削除を委譲する() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .remove_worktree("/r", "/wt", false)
            .unwrap();
        assert_eq!(
            *fake.removed_worktrees.lock(),
            vec![("/wt".to_string(), false)]
        );
    }

    #[test]
    fn test_read_onlyブランチカード一覧取得ではgcしない() {
        let fake = Arc::new(<FakeRepo as Default>::default());
        usecase(fake.clone())
            .list_branches_with_status_read_only("/r")
            .unwrap();
        assert!(fake.prune_calls.lock().is_empty());
    }
}
