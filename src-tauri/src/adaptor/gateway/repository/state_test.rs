use super::*;
use crate::adaptor::protocol::client as wire;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_dto::BranchCardDto;
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
};
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worktree::RepositoryStateNotifier;
use notify_debouncer_mini::DebouncedEventKind;
use prost::Message;
use std::sync::atomic::{AtomicUsize, Ordering};

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

    fn spawn_worker(&self, _future: RepositoryStateWorkerFuture) {}

    async fn sleep(&self, _duration: Duration) {}

    async fn scan(
        &self,
        _scanner: Arc<dyn RepositoryScanner>,
        _repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Err(RepositoryStateError::Watcher(
            "inert runtime does not scan".to_string(),
        ))
    }
}

struct EmptyScanner;

impl RepositoryScanner for EmptyScanner {
    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Ok(RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            branch_cards: Vec::new(),
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn prune_stale_branch_bases(
        &self,
        _repo_path: &str,
        _existing_branches: &[String],
    ) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

#[derive(Default)]
struct CountingNotifier {
    snapshot_committed: AtomicUsize,
}

impl RepositoryStateNotifier for CountingNotifier {
    fn snapshot_changed(&self, _notification: SnapshotNotification) {
        self.snapshot_committed.fetch_add(1, Ordering::SeqCst);
    }
}

fn event(path: &std::path::Path) -> DebouncedEvent {
    DebouncedEvent {
        path: path.to_path_buf(),
        kind: DebouncedEventKind::Any,
    }
}

fn state_with_notifier(notifier: Arc<CountingNotifier>) -> Arc<WorktreeState> {
    WorktreeState::new(
        "/repo".to_string(),
        Arc::new(EmptyScanner),
        notifier,
        Arc::new(InertRuntime),
        Duration::ZERO,
    )
}

#[test]
fn watcher_callbacks_only_invalidate_until_worker_commit() {
    let notifier = Arc::new(CountingNotifier::default());
    let state = state_with_notifier(notifier.clone());
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "content").unwrap();

    handle_file_events(state.as_ref(), vec![event(&file_path)]);
    handle_git_events(state.as_ref(), &[event(&PathBuf::from("/repo/.git/HEAD"))]);
    handle_git_events(state.as_ref(), &[event(&PathBuf::from("/repo/.git/index"))]);

    assert_eq!(state.requested_generation(), 3);
    assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 0);

    state.commit_snapshot(
        RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            branch_cards: vec![BranchCardDto {
                name: "main".to_string(),
                is_main_worktree: true,
                is_deleting: false,
                worktree_path: Some("/repo".to_string()),
                dirty_count: 0,
                is_merged: false,
                ahead: 0,
                behind: 0,
                has_upstream: false,
                base_ahead: 0,
            }],
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        },
        state.requested_generation(),
    );
    state.notify_snapshot_changed(InvalidateReason::git(true));

    assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 1);
}

#[test]
fn test_スキャン完了通知_gitとbranchとfileのみclientへ送る() {
    // Given
    let sink = Arc::new(PushSink::new());
    let mut receiver = sink.subscribe();
    let notifier = ClientRepositoryStateNotifier::new(
        sink,
        crate::usecase::state_subscription::StateSubscriptionPublisher::for_test(),
    );

    // When
    notifier.snapshot_changed(SnapshotNotification {
        worktree_paths: vec!["/repo".into()],
        file_watcher_ids: vec![7],
        reason: InvalidateReason::file(Some("/repo/file.txt".into())),
    });

    // Then
    let mut events = Vec::new();
    while let Ok(bytes) = receiver.try_recv() {
        events.push(wire::Push::decode(bytes.as_ref()).unwrap().event.unwrap());
    }
    assert_eq!(
        events,
        vec![
            wire::push::Event::GitStatusChanged(wire::GitStatusChangedEvent {
                repo_path: Some("/repo".into()),
            }),
            wire::push::Event::FileChange(wire::FileChangeEvent {
                watcher_id: Some(7),
                path: Some("/repo/file.txt".into()),
                kind: Some("change".into()),
            }),
        ]
    );
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
    use crate::usecase::repository_query_service::classify_branch_cards;
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
        let mut cards =
            super::super::branch_card::list_branches_with_status(repo_path.to_str().unwrap())
                .unwrap();
        gateway
            .include_deleting_worktrees(&root, &mut cards)
            .unwrap();
        let groups = classify_branch_cards(&root, &mut cards);
        let entries = repository
            .list_worktrees(repo_path.to_str().unwrap())
            .unwrap();
        // Then
        assert!(!cards.iter().any(|card| card.name == isolated.branch));
        assert_eq!(groups.working_areas.len(), 2);
        assert_eq!(entries.len(), groups.working_areas.len());
        for entry in entries {
            let card = groups
                .working_areas
                .iter()
                .find(|card| card.name == entry.branch)
                .unwrap();
            assert_eq!(card.worktree_path.as_deref(), Some(entry.path.as_str()));
        }
        let deleting: Vec<_> = cards.iter().filter(|card| card.is_deleting).collect();
        assert_eq!(deleting.len(), 1);
        assert_eq!(deleting[0].name, "feature");
        assert_eq!(
            deleting[0].worktree_path.as_deref(),
            Some(canonical_worktree.as_str())
        );
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
    let mut cards = Vec::new();
    gateway
        .include_deleting_worktrees(&root, &mut cards)
        .unwrap();
    assert_eq!(cards.len(), 1);
    assert!(cards[0].is_deleting);
    drop(deletion);
    let mut cards = Vec::new();
    gateway
        .include_deleting_worktrees(&root, &mut cards)
        .unwrap();
    assert!(cards.is_empty());
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
    let mut cards = super::super::branch_card::list_branches_with_status(repo_path).unwrap();
    gateway
        .include_deleting_worktrees(&root, &mut cards)
        .unwrap();

    // Then
    let entry = entries
        .iter()
        .find(|entry| entry.branch == "feature")
        .unwrap();
    let card = cards.iter().find(|card| card.name == "feature").unwrap();
    assert_eq!(created.path, entry.path);
    assert_eq!(card.worktree_path.as_deref(), Some(created.path.as_str()));
    assert_eq!(repository.get_main_repo_path(&created.path).unwrap(), root);
}

#[cfg(unix)]
#[test]
fn test_workspace一覧_パス識別失敗を呼び出し元へ返す() {
    // Given
    let repository = Arc::new(crate::adaptor::controller::wiring::build_repository_usecase());
    let gateway = RepositoryStateRepositoryGateway::new(repository);
    let mut cards = vec![BranchCardDto {
        name: "feature".into(),
        worktree_path: Some("/invalid\0path".into()),
        is_main_worktree: false,
        is_deleting: false,
        dirty_count: 0,
        is_merged: false,
        ahead: 0,
        behind: 0,
        has_upstream: false,
        base_ahead: 0,
    }];
    // When / Then
    assert!(gateway
        .include_deleting_worktrees("/repo", &mut cards)
        .is_err());
}
