use super::*;
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use notify_debouncer_mini::DebouncedEventKind;

struct InertSender;

impl RepositoryStateInvalidationSender for InertSender {
    fn send(&self, _reason: InvalidateReason) -> Result<(), ()> {
        Ok(())
    }
}

struct InertReceiver;

#[async_trait::async_trait]
impl RepositoryStateInvalidationReceiver for InertReceiver {
    async fn recv(&mut self) -> Option<InvalidateReason> {
        None
    }

    fn try_recv(&mut self) -> Option<InvalidateReason> {
        None
    }
}

struct InertRuntime;

#[async_trait::async_trait]
impl RepositoryStateWorkerRuntime for InertRuntime {
    fn invalidation_channel(
        &self,
    ) -> (
        Box<dyn RepositoryStateInvalidationSender>,
        Box<dyn RepositoryStateInvalidationReceiver>,
    ) {
        (Box::new(InertSender), Box::new(InertReceiver))
    }

    async fn scan(
        &self,
        _scanner: Arc<dyn RepositoryScanner>,
        _repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Err(RepositoryStateError::Watcher(
            "inert runtime does not scan".to_string(),
        ))
    }

    async fn scan_worktrees(
        &self,
        _scanner: Arc<dyn RepositoryScanner>,
        _repo_path: String,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        Err(RepositoryStateError::Watcher(
            "inert runtime does not scan".to_string(),
        ))
    }
}

struct EmptyScanner;

#[async_trait::async_trait]

impl RepositoryScanner for EmptyScanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Ok(RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn scan_worktrees(
        &self,
        _repo_path: &str,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        Ok(Vec::new())
    }

    fn prune_stale_branch_bases(&self, _repo_path: &str) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

fn event(path: &std::path::Path) -> DebouncedEvent {
    DebouncedEvent {
        path: path.to_path_buf(),
        kind: DebouncedEventKind::Any,
    }
}

fn state_with_subscriptions(
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> Arc<WorktreeState> {
    state_at("/repo", true, subscriptions)
}

fn state_at(
    path: &str,
    is_repository_root: bool,
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> Arc<WorktreeState> {
    WorktreeState::new(
        path.to_string(),
        is_repository_root,
        Arc::new(EmptyScanner),
        subscriptions,
        Arc::new(InertRuntime),
        tokio::sync::mpsc::unbounded_channel().0,
    )
}

#[test]
fn watcher_callbacks_only_invalidate_until_worker_commit() {
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let state = state_with_subscriptions(subscriptions);
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "content").unwrap();

    handle_file_events(state.as_ref(), vec![event(&file_path)]);
    let git_dir = Path::new("/repo/.git");
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/HEAD"))],
    );
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/index"))],
    );

    assert_eq!(state.requested_generation(), 3);
    assert!(changes.try_recv().is_err());

    state.commit_snapshot(
        RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        },
        state.requested_generation(),
    );
    state.notify_snapshot_changed();

    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Repository(vec!["/repo".into()])
    );
}

#[test]
fn test_ファイル監視_gitディレクトリの中の変化では変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/.git/worktrees/feature/index")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/src/main.rs")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}

#[test]
fn test_git監視_他のworktreeのindexの変化ではrootの変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_git_events(
        state.as_ref(),
        Path::new("/repo/.git"),
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
}

#[test]
fn test_git監視_linked_worktreeは自分のindexだけを読み直しrefの変化では読み直さない() {
    // Given
    let state = state_at(
        "/repo-worktrees/feature",
        false,
        crate::test_support::state_subscription::test_subscriptions(),
    );
    let git_dir = Path::new("/repo/.git/worktrees/feature");
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/HEAD"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}

#[cfg(unix)]
#[test]
fn test_worktree削除一覧_repositoryの別表記も同じrootへ解決する() {
    // Given
    let (dir, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("repository");
    std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
    let gateway = RepositoryStateRepositoryGateway::new(Arc::new(
        crate::adaptor::controller::wiring::build_repository_usecase(),
    ));
    // When
    let root = gateway.main_repo_path(alias.to_str().unwrap()).unwrap();
    // Then
    assert_eq!(root, dir.path().canonicalize().unwrap().to_string_lossy());
    assert_eq!(
        root,
        gateway
            .main_repo_path(dir.path().to_str().unwrap())
            .unwrap()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_workspace一覧_別表記の隔離worktreeを除外し削除中の通常worktreeを保持する() {
    // Given
    let parent = tempfile::tempdir().unwrap();
    let real = parent.path().canonicalize().unwrap();
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let root = real.join("repo");
    let repo = git2::Repository::init(&root).unwrap();
    crate::test_support::git::create_initial_commit(&repo);
    let isolated = crate::domain::workflow::IsolatedWorktree::for_attempt(
        alias.join("repo").to_str().unwrap(),
        "node",
        1,
    );
    let ordinary = alias.join("repo-worktrees/feature");
    std::fs::create_dir_all(Path::new(&isolated.path).parent().unwrap()).unwrap();
    for (name, branch_name, path) in [
        (
            "isolated",
            isolated.branch.as_str(),
            Path::new(&isolated.path),
        ),
        ("feature", "feature", ordinary.as_path()),
    ] {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let branch = repo
            .branch(branch_name, &head, false)
            .unwrap()
            .into_reference();
        let mut options = git2::WorktreeAddOptions::new();
        options.reference(Some(&branch));
        repo.worktree(name, path, Some(&options)).unwrap();
        std::fs::write(
            repo.path().join("worktrees").join(name).join("gitdir"),
            format!("{}\n", path.join(".git").display()),
        )
        .unwrap();
    }
    let repository = Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
    let gateway = RepositoryStateRepositoryGateway::new(repository.clone());
    let operations = repository.worktree_operations();
    let canonical_worktree = ordinary
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut deletion = operations.delete(&canonical_worktree).await.unwrap();
    deletion
        .accept(
            crate::domain::repository::worktree_operation::WorktreeDeletionTarget {
                repository_root: repository.get_main_repo_path(&canonical_worktree).unwrap(),
                path: canonical_worktree.clone(),
                branch: None,
            },
        )
        .unwrap();
    for repo_path in [root, alias.join("repo")] {
        // When
        let root = gateway.main_repo_path(repo_path.to_str().unwrap()).unwrap();
        let rows = repository.with_deleting_worktrees(
            &root,
            repository
                .list_working_worktrees(repo_path.to_str().unwrap())
                .unwrap(),
        );
        let entries = repository
            .list_worktrees(repo_path.to_str().unwrap())
            .unwrap();
        // Then
        assert!(!rows
            .iter()
            .any(|(worktree, _)| worktree.branch == isolated.branch));
        assert_eq!(rows.len(), 2);
        assert_eq!(entries.len(), rows.len());
        for entry in entries {
            let (worktree, _) = rows
                .iter()
                .find(|(worktree, _)| worktree.branch == entry.branch)
                .unwrap();
            assert_eq!(worktree.path, entry.path);
        }
        let deleting: Vec<_> = rows.iter().filter(|(_, deleting)| *deleting).collect();
        assert_eq!(deleting.len(), 1);
        assert_eq!(deleting[0].0.branch, "feature");
        assert_eq!(deleting[0].0.path, canonical_worktree);
        assert!(operations.mutate(&canonical_worktree).is_err());
    }
    // Git 管理情報と実体が失われた後も同じ削除対象を読み出す。
    super::super::worktree::remove_worktree(
        real.join("repo").to_str().unwrap(),
        &canonical_worktree,
        false,
    )
    .unwrap();
    let root = real.join("repo").to_string_lossy().into_owned();
    let rows = repository.with_deleting_worktrees(&root, Vec::new());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1);
    drop(deletion);
    assert!(repository
        .with_deleting_worktrees(&root, Vec::new())
        .is_empty());
    assert!(operations.mutate(&canonical_worktree).is_ok());
}

#[cfg(unix)]
#[test]
fn test_workspace一覧_別表記で作成したworktreeの選択パスが一覧と一致する() {
    // Given
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("repo");
    let repo = git2::Repository::init(&root).unwrap();
    crate::test_support::git::create_initial_commit(&repo);
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(parent.path(), &alias).unwrap();
    let repo_path = alias.join("repo");
    let repo_path = repo_path.to_str().unwrap();
    let repository = Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
    let gateway = RepositoryStateRepositoryGateway::new(repository.clone());

    // When
    let created = repository
        .create_worktree(repo_path, "feature", true, None)
        .unwrap();
    let entries = repository.list_worktrees(repo_path).unwrap();
    let root = gateway.main_repo_path(repo_path).unwrap();
    let worktrees = repository.list_working_worktrees(repo_path).unwrap();

    // Then
    let entry = entries
        .iter()
        .find(|entry| entry.branch == "feature")
        .unwrap();
    let worktree = worktrees
        .iter()
        .find(|worktree| worktree.branch == "feature")
        .unwrap();
    assert_eq!(created.path, entry.path);
    assert_eq!(worktree.path, created.path);
    assert_eq!(repository.get_main_repo_path(&created.path).unwrap(), root);
}

#[tokio::test]
async fn test_repository走査の期限切れ_旧走査を回収して同じ対象と他対象を走査できる() {
    // Given
    let (dir, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    let scanner = Arc::new(super::super::scanner::DefaultRepositoryScanner::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(crate::adaptor::controller::wiring::build_code_usecase()),
    ));
    let path = dir.path().to_str().unwrap().to_owned();
    let scan_lock = Arc::new(tokio::sync::Mutex::new(()));
    let attempt_scanner = scanner.clone();
    let attempt_path = path.clone();
    let attempt_lock = scan_lock.clone();
    // When
    super::super::super::shared::background_worker::background_worker_tests::assert_expired_releases(Box::pin(async move {
        let _scan = attempt_lock.lock().await;
        crate::adaptor::controller::repository_scan::RepositoryScanWorkerRuntime::new().scan(attempt_scanner, attempt_path).await.map_err(|error| crate::usecase::failure::WorkFailure::from_error(&error))?;
        Ok(())
    })).await;
    // Then
    assert!(scan_lock.try_lock().is_ok());
    crate::adaptor::controller::repository_scan::RepositoryScanWorkerRuntime::new()
        .scan(scanner.clone(), path)
        .await
        .unwrap();
    let (other, repo) = crate::test_support::git::create_test_repo();
    crate::test_support::git::create_initial_commit(&repo);
    crate::adaptor::controller::repository_scan::RepositoryScanWorkerRuntime::new()
        .scan(scanner, other.path().to_str().unwrap().into())
        .await
        .unwrap();
}
