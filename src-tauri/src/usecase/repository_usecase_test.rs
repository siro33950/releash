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
