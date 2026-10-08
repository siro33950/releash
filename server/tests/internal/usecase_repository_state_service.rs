use releashd::test_support::integration::fixtures::repository_state_CountingScanner as CountingScanner;
use releashd::test_support::integration::fixtures::repository_state_TestRepositoryStateRepository as TestRepositoryStateRepository;
use releashd::test_support::integration::fixtures::repository_state_counting_service as counting_service;
pub(crate) mod tests {
    fn test_service(scanner: Arc<EmptyScanner>) -> RepositoryStateService {
        RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            releashd::test_support::integration::subscriptions::test_subscriptions(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            releashd::test_support::integration::subscriptions::repository_driver(),
        )
    }
    fn test_service_with_notifier(
        scanner: Arc<EmptyScanner>,
        subscriptions: releashd::test_support::integration::subscriptions::StateSubscriptionUsecase,
    ) -> RepositoryStateService {
        RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            subscriptions,
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            releashd::test_support::integration::subscriptions::repository_driver(),
        )
    }
    use super::*;
    use releashd::test_support::integration::fixtures::repository_state_EmptyScanner as EmptyScanner;

    use releashd::test_support::integration::platform::RepositoryStateError;
    use releashd::test_support::integration::platform::RepositoryStateService;
    use releashd::test_support::integration::platform::RepositoryStateWatcher;
    use releashd::test_support::integration::platform::WorktreeState;
    use std::sync::Arc;

    use crate::usecase_repository_state_test_helpers_runtime::CanonicalWorktreePathNormalizer;
    use releashd::test_support::integration::platform::FileStatusDto;
    use releashd::test_support::integration::platform::IdentityWorktreePathNormalizer;
    use releashd::test_support::integration::platform::InvalidateReason;
    use releashd::test_support::integration::platform::NoopRepositoryStateWatcher;
    use releashd::test_support::integration::platform::TestRepositoryStateWorkerRuntime;
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[derive(Default)]
    struct CountingRepositoryStateWatcher {
        next_id: AtomicU64,
        started_paths: parking_lot::Mutex<Vec<String>>,
    }

    impl CountingRepositoryStateWatcher {
        fn start_count(&self) -> usize {
            self.started_paths.lock().len()
        }

        fn started_paths(&self) -> Vec<String> {
            self.started_paths.lock().clone()
        }
    }

    impl RepositoryStateWatcher for CountingRepositoryStateWatcher {
        fn next_watcher_id(&self) -> u64 {
            self.next_id.fetch_add(1, Ordering::SeqCst) + 1
        }

        fn start_watchers(
            &self,
            state: Arc<WorktreeState>,
        ) -> Result<
            Box<dyn releashd::test_support::integration::platform::RepositoryStateWatchSession>,
            RepositoryStateError,
        > {
            self.started_paths
                .lock()
                .push(state.worktree_path().to_string());
            Ok(Box::new(()))
        }
    }

    pub(crate) fn watching_service() -> RepositoryStateService {
        test_service(Arc::new(EmptyScanner))
    }

    use releashd::test_support::integration::subscriptions::CapturingNotifier;

    #[tokio::test]
    pub async fn unmanaged_read_returns_ephemeral_snapshot_without_creating_worktree_or_watcher() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "changed.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher.clone());
        let dir = tempfile::TempDir::new().unwrap();

        let snapshot = service.get_snapshot(dir.path().to_str().unwrap()).unwrap();

        assert_eq!(snapshot.version, 0);
        assert!(!snapshot.flags.loading);
        assert_eq!(snapshot.status.len(), 1);
        assert_eq!(scanner.scan_count(), 1);
        assert_eq!(service.worktree_count(), 0);
        assert_eq!(watcher.start_count(), 0);
    }

    #[tokio::test]
    pub async fn unmanaged_reads_for_distinct_paths_do_not_accumulate_workers_or_watchers() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher.clone());
        let dirs = [
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
        ];

        for dir in &dirs {
            service.get_snapshot(dir.path().to_str().unwrap()).unwrap();
        }

        assert_eq!(scanner.scan_count(), 3);
        assert_eq!(service.worktree_count(), 0);
        assert_eq!(watcher.start_count(), 0);
    }

    #[tokio::test]
    pub async fn managed_worktree_read_uses_cached_snapshot_without_rescanning() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "worker.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        for _ in 0..100 {
            if service.get_snapshot(path).unwrap().version >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let scan_count = scanner.scan_count();

        let snapshot = service.get_snapshot(path).unwrap();

        assert!(snapshot.version >= 1);
        assert_eq!(snapshot.status[0].path, "worker.txt");
        assert_eq!(scanner.scan_count(), scan_count);
        assert_eq!(service.worktree_count(), 1);
    }

    #[tokio::test]
    pub async fn snapshot_dtos_are_derived_from_same_cached_version() {
        // Given
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "changed.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let service =
            counting_service(scanner, Arc::new(CountingRepositoryStateWatcher::default()));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        for _ in 0..100 {
            if service.get_snapshot(path).unwrap().version >= 1 {
                break;
            }
            // When
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let status = service.get_snapshot(path).unwrap();
        let diff_stats = service.get_snapshot(path).unwrap();
        let head_tree = service.get_snapshot(path).unwrap();

        // Then
        assert!(status.version >= 1);
        assert_eq!(diff_stats.version, status.version);
        assert_eq!(head_tree.version, status.version);
        assert_eq!(service.dirty_count(path).value, Some(status.status.len()));
    }

    #[tokio::test]
    pub async fn watcher_creation_happens_only_through_subscribe_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let (dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        let path = dir.path().to_str().unwrap();

        service.get_snapshot(path).unwrap();
        service.get_snapshot(path).unwrap();
        service.get_snapshot(path).unwrap();

        assert_eq!(watcher.start_count(), 0);
        assert_eq!(service.worktree_count(), 0);

        service.start_git_dir_watching(path).unwrap();
        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);

        let git_dir = tempfile::TempDir::new().unwrap();
        let git_path = git_dir.path().to_str().unwrap();
        service.start_git_dir_watching(git_path).unwrap();
        assert_eq!(watcher.start_count(), 2);
        assert_eq!(service.worktree_count(), 2);
    }

    #[tokio::test]
    pub async fn same_canonical_worktree_multiple_subscriptions_start_watchers_once() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        service.subscribe(path).unwrap();
        service.subscribe(path).unwrap();

        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);
    }

    #[tokio::test]
    pub async fn different_worktrees_start_one_watcher_each_without_duplicate_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let first = tempfile::TempDir::new().unwrap();
        let second = tempfile::TempDir::new().unwrap();
        let first_path = first.path().to_str().unwrap();
        let second_path = second.path().to_str().unwrap();

        service.subscribe(first_path).unwrap();
        service.subscribe(second_path).unwrap();

        let started_paths = watcher.started_paths();
        assert_eq!(started_paths.len(), 2);
        assert!(started_paths.contains(&first_path.to_string()));
        assert!(started_paths.contains(&second_path.to_string()));
    }

    #[test]
    pub fn real_repo_dirty_count_matches_legacy_count_for_untracked_rename_and_typechange() {
        let (dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        crate::test_support_git::add_and_commit(&repo, "rename-old.txt", "old", "add old");
        crate::test_support_git::add_and_commit(&repo, "typechange", "file", "add typechange");

        std::fs::create_dir_all(dir.path().join("untracked-dir").join("nested")).unwrap();
        std::fs::write(
            dir.path()
                .join("untracked-dir")
                .join("nested")
                .join("file.txt"),
            "new",
        )
        .unwrap();
        std::fs::rename(
            dir.path().join("rename-old.txt"),
            dir.path().join("rename-new.txt"),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("typechange")).unwrap();
        std::fs::create_dir(dir.path().join("typechange")).unwrap();
        std::fs::write(dir.path().join("typechange").join("child.txt"), "child").unwrap();

        let path = dir.path().to_str().unwrap();
        let status = releashd::test_support::integration::repository::get_git_status(path).unwrap();
        let scanner = Arc::new(CountingScanner::with_status(
            status.into_iter().map(Into::into).collect(),
        ));
        let service = RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            releashd::test_support::integration::subscriptions::test_subscriptions(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
            releashd::test_support::integration::subscriptions::repository_driver(),
        );

        let legacy =
            releashd::test_support::integration::repository::get_worktree_dirty_count(path)
                .unwrap();
        let snapshot_count = service.get_snapshot(path).unwrap().status.len() as u32;

        assert_eq!(snapshot_count, legacy);
    }

    #[tokio::test]
    pub async fn subscriptions_share_state_but_get_distinct_release_ids() {
        let service = test_service(Arc::new(EmptyScanner));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        let first = service.subscribe(path).unwrap();
        let second = service.subscribe(path).unwrap();

        assert_ne!(first, second);
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(first).unwrap());
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(second).unwrap());
        assert_eq!(service.worktree_count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    pub async fn canonical_state_notifies_each_subscriber_path_alias() {
        let scanner = Arc::new(EmptyScanner);
        let subscriptions =
            releashd::test_support::integration::subscriptions::test_subscriptions();
        let notifier = Arc::new(CapturingNotifier::repositories(&subscriptions));
        let service = test_service_with_notifier(scanner, subscriptions);
        let dir = tempfile::TempDir::new().unwrap();
        let alias_parent = tempfile::TempDir::new().unwrap();
        let alias = alias_parent.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();

        let original_path = dir.path().to_str().unwrap();
        let alias_path = alias.to_str().unwrap();
        service.subscribe(original_path).unwrap();
        service.subscribe(alias_path).unwrap();
        let state = service.ensure_watching(original_path).unwrap();
        notifier.take();

        state.invalidate(InvalidateReason::change());

        for _ in 0..100 {
            let notifications = notifier.take();
            if let Some(committed) = notifications.first() {
                assert!(committed.iter().any(|path| path == original_path));
                assert!(committed.iter().any(|path| path == alias_path));
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("timed out waiting for alias notification");
    }
}
