use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use crate::usecase::repository_dto::{BranchCardDto, FileDiffStatDto, FileStatusDto};

use super::error::RepositoryStateError;
use super::runtime::{RepositoryStateWorkerRuntime, WorktreePathNormalizer};
use super::scanner::RepositoryScanner;
use super::snapshot::{
    RepositoryBranchCardsSnapshotDto, RepositoryDiffStatsSnapshotDto,
    RepositoryHeadDiffFileTreeSnapshotDto, RepositorySnapshot, RepositoryStatusSnapshotDto,
};
use super::worktree::WatchSubscriptionKind;
use super::worktree::{RepositoryStateNotifier, RepositoryStateWatcher, WorktreeState};

const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(300);

pub trait RepositoryStateRepository: Send + Sync {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError>;
}

pub struct RepositoryStateService {
    repository: Arc<dyn RepositoryStateRepository>,
    scanner: Arc<dyn RepositoryScanner>,
    notifier: Arc<dyn RepositoryStateNotifier>,
    watcher: Arc<dyn RepositoryStateWatcher>,
    runtime: Arc<dyn RepositoryStateWorkerRuntime>,
    path_normalizer: Arc<dyn WorktreePathNormalizer>,
    debounce: Duration,
    worktrees: RwLock<HashMap<PathBuf, Arc<WorktreeState>>>,
}

impl RepositoryStateService {
    pub fn new(
        repository: Arc<dyn RepositoryStateRepository>,
        scanner: Arc<dyn RepositoryScanner>,
        notifier: Arc<dyn RepositoryStateNotifier>,
        watcher: Arc<dyn RepositoryStateWatcher>,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        path_normalizer: Arc<dyn WorktreePathNormalizer>,
    ) -> Self {
        Self::new_with_scanner(
            repository,
            scanner,
            notifier,
            watcher,
            runtime,
            path_normalizer,
            DEFAULT_DEBOUNCE,
        )
    }

    pub fn new_with_scanner(
        repository: Arc<dyn RepositoryStateRepository>,
        scanner: Arc<dyn RepositoryScanner>,
        notifier: Arc<dyn RepositoryStateNotifier>,
        watcher: Arc<dyn RepositoryStateWatcher>,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        path_normalizer: Arc<dyn WorktreePathNormalizer>,
        debounce: Duration,
    ) -> Self {
        Self {
            repository,
            scanner,
            notifier,
            watcher,
            runtime,
            path_normalizer,
            debounce,
            worktrees: RwLock::new(HashMap::new()),
        }
    }

    pub fn start_file_watching_if_repository(
        &self,
        path: &str,
    ) -> Result<Option<u64>, RepositoryStateError> {
        if self.repository.main_repo_path(path).is_err() {
            return Ok(None);
        }
        Ok(Some(self.subscribe(path, WatchSubscriptionKind::File)?))
    }

    pub fn start_git_dir_watching(&self, repo_path: &str) -> Result<u64, RepositoryStateError> {
        self.subscribe(repo_path, WatchSubscriptionKind::Git)
    }

    pub fn get_snapshot(
        &self,
        worktree_path: &str,
    ) -> Result<Arc<RepositorySnapshot>, RepositoryStateError> {
        let key = self.canonical_worktree_key(worktree_path)?;
        if let Some(existing) = self.worktrees.read().get(&key) {
            return Ok(existing.snapshot_for_read());
        }

        let canonical_path = key.to_string_lossy().to_string();
        let snapshot = self.scanner.scan(&canonical_path)?.into_snapshot(0);
        Ok(Arc::new(snapshot))
    }

    pub fn get_status(
        &self,
        worktree_path: &str,
        include_ignored: bool,
    ) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
        if include_ignored {
            return self.scanner.status_with_ignored(worktree_path);
        }
        Ok(self.get_snapshot(worktree_path)?.status.clone())
    }

    pub fn get_status_snapshot(
        &self,
        worktree_path: &str,
    ) -> Result<RepositoryStatusSnapshotDto, RepositoryStateError> {
        let snapshot = self.get_snapshot(worktree_path)?;
        Ok(RepositoryStatusSnapshotDto::from_snapshot(
            snapshot.as_ref(),
        ))
    }

    pub fn get_diff_stats(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<FileDiffStatDto>, RepositoryStateError> {
        Ok(self.get_snapshot(worktree_path)?.diff_stats.clone())
    }

    pub fn get_diff_stats_snapshot(
        &self,
        worktree_path: &str,
    ) -> Result<RepositoryDiffStatsSnapshotDto, RepositoryStateError> {
        let snapshot = self.get_snapshot(worktree_path)?;
        Ok(RepositoryDiffStatsSnapshotDto::from_snapshot(
            snapshot.as_ref(),
        ))
    }

    pub fn get_head_diff_file_tree_snapshot(
        &self,
        worktree_path: &str,
    ) -> Result<RepositoryHeadDiffFileTreeSnapshotDto, RepositoryStateError> {
        let snapshot = self.get_snapshot(worktree_path)?;
        Ok(RepositoryHeadDiffFileTreeSnapshotDto::from_snapshot(
            snapshot.as_ref(),
        ))
    }

    pub fn list_branches_with_status(
        &self,
        repo_path: &str,
    ) -> Result<Vec<BranchCardDto>, RepositoryStateError> {
        Ok(self.get_snapshot(repo_path)?.branch_cards.clone())
    }

    pub fn list_branches_with_status_snapshot(
        &self,
        repo_path: &str,
    ) -> Result<RepositoryBranchCardsSnapshotDto, RepositoryStateError> {
        let snapshot = self.get_snapshot(repo_path)?;
        Ok(RepositoryBranchCardsSnapshotDto::from_snapshot(
            snapshot.as_ref(),
        ))
    }

    pub fn get_worktree_dirty_count(
        &self,
        worktree_path: &str,
    ) -> Result<u32, RepositoryStateError> {
        Ok(self.get_snapshot(worktree_path)?.status.len() as u32)
    }

    pub fn stop_watching(&self, watcher_id: u64) -> Result<bool, RepositoryStateError> {
        let mut worktrees = self.worktrees.write();
        let mut empty_key = None;
        let mut found = false;

        for (key, state) in worktrees.iter() {
            if state.release_subscription(watcher_id) {
                found = true;
                if state.subscriber_count() == 0 {
                    empty_key = Some(key.clone());
                }
                break;
            }
        }

        if let Some(key) = empty_key {
            if let Some(state) = worktrees.remove(&key) {
                state.shutdown();
            }
        }

        Ok(found)
    }

    fn subscribe(
        &self,
        worktree_path: &str,
        kind: WatchSubscriptionKind,
    ) -> Result<u64, RepositoryStateError> {
        let subscription_id = self.watcher.next_watcher_id();
        self.ensure_watching_with_subscription(worktree_path, Some((subscription_id, kind)))?;
        Ok(subscription_id)
    }

    #[cfg(test)]
    fn ensure_watching(
        &self,
        worktree_path: &str,
    ) -> Result<Arc<WorktreeState>, RepositoryStateError> {
        self.ensure_watching_with_subscription(worktree_path, None)
    }

    fn ensure_watching_with_subscription(
        &self,
        worktree_path: &str,
        subscription: Option<(u64, WatchSubscriptionKind)>,
    ) -> Result<Arc<WorktreeState>, RepositoryStateError> {
        let key = self.canonical_worktree_key(worktree_path)?;
        if let Some(existing) = self.worktrees.read().get(&key) {
            if let Some((id, kind)) = subscription {
                existing.add_subscription(id, kind, worktree_path.to_string());
            }
            return Ok(existing.clone());
        }

        let mut worktrees = self.worktrees.write();
        if let Some(existing) = worktrees.get(&key) {
            if let Some((id, kind)) = subscription {
                existing.add_subscription(id, kind, worktree_path.to_string());
            }
            return Ok(existing.clone());
        }

        let canonical_path = key.to_string_lossy().to_string();
        let state = WorktreeState::new(
            canonical_path,
            self.scanner.clone(),
            self.notifier.clone(),
            self.runtime.clone(),
            self.debounce,
        );
        if let Some((id, kind)) = subscription {
            state.add_subscription(id, kind, worktree_path.to_string());
        }
        if let Err(err) = state.start_watchers(self.watcher.as_ref()) {
            state.shutdown();
            return Err(err);
        }
        worktrees.insert(key, state.clone());
        Ok(state)
    }

    #[cfg(test)]
    fn ensure_for_tests(&self, worktree_path: &str) -> Arc<WorktreeState> {
        let key = PathBuf::from(worktree_path);
        if let Some(existing) = self.worktrees.read().get(&key) {
            return existing.clone();
        }
        let mut worktrees = self.worktrees.write();
        if let Some(existing) = worktrees.get(&key) {
            return existing.clone();
        }
        let state = WorktreeState::new(
            worktree_path.to_string(),
            self.scanner.clone(),
            self.notifier.clone(),
            self.runtime.clone(),
            self.debounce,
        );
        worktrees.insert(key, state.clone());
        state
    }

    #[cfg(test)]
    fn worktree_count(&self) -> usize {
        self.worktrees.read().len()
    }

    fn canonical_worktree_key(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
        self.path_normalizer.normalize(worktree_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::repository_dto::{BranchCardDto, FileDiffStatDto, FileStatusDto};
    use crate::usecase::repository_state::runtime::tests_support::{
        CanonicalWorktreePathNormalizer, IdentityWorktreePathNormalizer,
        TestRepositoryStateWorkerRuntime,
    };
    use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    use crate::usecase::repository_state::worker::InvalidateReason;
    use crate::usecase::repository_state::worktree::{
        NoopRepositoryStateNotifier, NoopRepositoryStateWatcher, SnapshotNotification,
    };
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    struct TestRepositoryStateRepository;

    impl RepositoryStateRepository for TestRepositoryStateRepository {
        fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
            Ok(path.to_string())
        }
    }

    struct EmptyScanner {
        ignored_calls: AtomicUsize,
    }

    impl RepositoryScanner for EmptyScanner {
        fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Ok(RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                branch_cards: Vec::new(),
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            })
        }

        fn status_with_ignored(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
            self.ignored_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![FileStatusDto {
                path: "ignored.txt".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            }])
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
    struct CountingScanner {
        scans: AtomicUsize,
        prunes: parking_lot::Mutex<Vec<Vec<String>>>,
        status: parking_lot::Mutex<Vec<FileStatusDto>>,
        diff_stats: parking_lot::Mutex<Vec<FileDiffStatDto>>,
        branch_cards: parking_lot::Mutex<Vec<BranchCardDto>>,
    }

    impl CountingScanner {
        fn with_status(status: Vec<FileStatusDto>) -> Self {
            Self {
                status: parking_lot::Mutex::new(status),
                ..Self::default()
            }
        }

        fn scan_count(&self) -> usize {
            self.scans.load(Ordering::SeqCst)
        }

        fn prune_calls(&self) -> Vec<Vec<String>> {
            self.prunes.lock().clone()
        }

        fn set_branch_cards(&self, branch_cards: Vec<BranchCardDto>) {
            *self.branch_cards.lock() = branch_cards;
        }
    }

    impl RepositoryScanner for CountingScanner {
        fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            Ok(RepositorySnapshotParts {
                status: self.status.lock().clone(),
                diff_stats: self.diff_stats.lock().clone(),
                branch_cards: self.branch_cards.lock().clone(),
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            })
        }

        fn status_with_ignored(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
            let mut status = self.status.lock().clone();
            status.push(FileStatusDto {
                path: "ignored.txt".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            });
            Ok(status)
        }

        fn prune_stale_branch_bases(
            &self,
            _repo_path: &str,
            existing_branches: &[String],
        ) -> Result<(), RepositoryStateError> {
            self.prunes.lock().push(existing_branches.to_vec());
            Ok(())
        }
    }

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
            Box<dyn crate::usecase::repository_state::worktree::RepositoryStateWatchSession>,
            RepositoryStateError,
        > {
            self.started_paths
                .lock()
                .push(state.worktree_path().to_string());
            Ok(Box::new(()))
        }
    }

    fn test_service(scanner: Arc<EmptyScanner>) -> RepositoryStateService {
        RepositoryStateService::new_with_scanner(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            Arc::new(NoopRepositoryStateNotifier),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            Duration::ZERO,
        )
    }

    fn test_service_with_notifier(
        scanner: Arc<EmptyScanner>,
        notifier: Arc<dyn RepositoryStateNotifier>,
    ) -> RepositoryStateService {
        RepositoryStateService::new_with_scanner(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            notifier,
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            Duration::ZERO,
        )
    }

    fn counting_service(
        scanner: Arc<CountingScanner>,
        watcher: Arc<dyn RepositoryStateWatcher>,
    ) -> RepositoryStateService {
        RepositoryStateService::new_with_scanner(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            Arc::new(NoopRepositoryStateNotifier),
            watcher,
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
            Duration::ZERO,
        )
    }

    #[derive(Default)]
    struct CapturingNotifier {
        notifications: parking_lot::Mutex<Vec<SnapshotNotification>>,
    }

    impl CapturingNotifier {
        fn take(&self) -> Vec<SnapshotNotification> {
            std::mem::take(&mut *self.notifications.lock())
        }
    }

    impl RepositoryStateNotifier for CapturingNotifier {
        fn snapshot_changed(&self, notification: SnapshotNotification) {
            self.notifications.lock().push(notification);
        }
    }

    #[tokio::test]
    async fn same_worktree_reuses_one_state() {
        let service = test_service(Arc::new(EmptyScanner {
            ignored_calls: AtomicUsize::new(0),
        }));

        let first = service.ensure_for_tests("/repo");
        let second = service.ensure_for_tests("/repo");

        assert_eq!(service.worktree_count(), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn unmanaged_read_returns_ephemeral_snapshot_without_creating_worktree_or_watcher() {
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
    async fn unmanaged_reads_for_distinct_paths_do_not_accumulate_workers_or_watchers() {
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
    async fn managed_worktree_read_uses_cached_snapshot_without_rescanning() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "worker.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service
            .subscribe(path, WatchSubscriptionKind::File)
            .unwrap();
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
    async fn snapshot_dtos_are_derived_from_same_cached_version() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "changed.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        scanner.set_branch_cards(vec![BranchCardDto {
            name: "main".to_string(),
            is_main_worktree: true,
            worktree_path: Some("/repo".to_string()),
            dirty_count: 1,
            is_merged: false,
            ahead: 0,
            behind: 0,
            has_upstream: false,
            base_ahead: 0,
        }]);
        let service =
            counting_service(scanner, Arc::new(CountingRepositoryStateWatcher::default()));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service
            .subscribe(path, WatchSubscriptionKind::File)
            .unwrap();
        for _ in 0..100 {
            if service.get_status_snapshot(path).unwrap().version >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let status = service.get_status_snapshot(path).unwrap();
        let diff_stats = service.get_diff_stats_snapshot(path).unwrap();
        let branch_cards = service.list_branches_with_status_snapshot(path).unwrap();
        let head_tree = service.get_head_diff_file_tree_snapshot(path).unwrap();
        let dirty_count = service.get_worktree_dirty_count(path).unwrap();

        assert!(status.version >= 1);
        assert_eq!(diff_stats.version, status.version);
        assert_eq!(branch_cards.version, status.version);
        assert_eq!(head_tree.version, status.version);
        assert_eq!(dirty_count as usize, status.status.len());
    }

    #[tokio::test]
    async fn watcher_creation_happens_only_through_subscribe_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let path = dir.path().to_str().unwrap();

        service.get_status_snapshot(path).unwrap();
        service.get_diff_stats_snapshot(path).unwrap();
        service.get_head_diff_file_tree_snapshot(path).unwrap();

        assert_eq!(watcher.start_count(), 0);
        assert_eq!(service.worktree_count(), 0);

        service.start_file_watching_if_repository(path).unwrap();
        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);

        let git_dir = tempfile::TempDir::new().unwrap();
        let git_path = git_dir.path().to_str().unwrap();
        service.start_git_dir_watching(git_path).unwrap();
        assert_eq!(watcher.start_count(), 2);
        assert_eq!(service.worktree_count(), 2);
    }

    #[tokio::test]
    async fn file_change_in_one_worktree_does_not_update_other_worktree_snapshot_version() {
        let scanner = Arc::new(CountingScanner::default());
        let service = counting_service(scanner, Arc::new(NoopRepositoryStateWatcher));

        let first = service.ensure_for_tests("/repo-one");
        let second = service.ensure_for_tests("/repo-two");
        first.commit_snapshot(
            RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                branch_cards: Vec::new(),
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            },
            0,
        );
        second.commit_snapshot(
            RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                branch_cards: Vec::new(),
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            },
            0,
        );

        first.invalidate(InvalidateReason::file(Some("a.txt".to_string())));

        for _ in 0..100 {
            if first.snapshot_for_read().version >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(first.snapshot_for_read().version >= 2);
        assert_eq!(second.snapshot_for_read().version, 1);
    }

    #[tokio::test]
    async fn same_canonical_worktree_multiple_subscriptions_start_watchers_once() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service
            .subscribe(path, WatchSubscriptionKind::File)
            .unwrap();
        service.subscribe(path, WatchSubscriptionKind::Git).unwrap();
        service
            .subscribe(path, WatchSubscriptionKind::File)
            .unwrap();

        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);
    }

    #[tokio::test]
    async fn different_worktrees_start_one_watcher_each_without_duplicate_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let first = tempfile::TempDir::new().unwrap();
        let second = tempfile::TempDir::new().unwrap();
        let first_path = first.path().to_str().unwrap();
        let second_path = second.path().to_str().unwrap();

        service
            .subscribe(first_path, WatchSubscriptionKind::File)
            .unwrap();
        service
            .subscribe(second_path, WatchSubscriptionKind::Git)
            .unwrap();

        let started_paths = watcher.started_paths();
        assert_eq!(started_paths.len(), 2);
        assert!(started_paths.contains(&first_path.to_string()));
        assert!(started_paths.contains(&second_path.to_string()));
    }

    #[test]
    fn dirty_count_uses_default_snapshot_status_and_excludes_ignored_opt_in_status() {
        let scanner = Arc::new(CountingScanner::with_status(vec![
            FileStatusDto {
                path: "modified.txt".to_string(),
                index_status: "none".to_string(),
                worktree_status: "modified".to_string(),
            },
            FileStatusDto {
                path: "staged.txt".to_string(),
                index_status: "modified".to_string(),
                worktree_status: "none".to_string(),
            },
        ]));
        let service = counting_service(scanner.clone(), Arc::new(NoopRepositoryStateWatcher));

        assert_eq!(service.get_worktree_dirty_count("/repo").unwrap(), 2);
        assert_eq!(service.get_status("/repo", true).unwrap().len(), 3);
    }

    #[test]
    fn list_branches_with_status_is_pure_read_and_does_not_prune() {
        let scanner = Arc::new(CountingScanner::default());
        scanner.set_branch_cards(vec![BranchCardDto {
            name: "main".to_string(),
            is_main_worktree: true,
            worktree_path: Some("/repo".to_string()),
            dirty_count: 0,
            is_merged: false,
            ahead: 0,
            behind: 0,
            has_upstream: false,
            base_ahead: 0,
        }]);
        let service = counting_service(scanner.clone(), Arc::new(NoopRepositoryStateWatcher));

        let cards = service.list_branches_with_status("/repo").unwrap();

        assert_eq!(cards.len(), 1);
        assert!(scanner.prune_calls().is_empty());
    }

    #[test]
    fn real_repo_dirty_count_matches_legacy_count_for_untracked_rename_and_typechange() {
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        crate::test_support::git::add_and_commit(&repo, "rename-old.txt", "old", "add old");
        crate::test_support::git::add_and_commit(&repo, "typechange", "file", "add typechange");

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
        let status = crate::adaptor::gateway::repository::status::get_git_status(path)
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect();
        let scanner = Arc::new(CountingScanner::with_status(status));
        let service = RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            Arc::new(NoopRepositoryStateNotifier),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
        );

        let legacy =
            crate::adaptor::gateway::repository::worktree::get_worktree_dirty_count(path).unwrap();
        let snapshot_count = service.get_worktree_dirty_count(path).unwrap();

        assert_eq!(snapshot_count, legacy);
    }

    #[tokio::test]
    async fn subscriptions_share_state_but_get_distinct_release_ids() {
        let service = test_service(Arc::new(EmptyScanner {
            ignored_calls: AtomicUsize::new(0),
        }));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        let first = service
            .subscribe(path, WatchSubscriptionKind::File)
            .unwrap();
        let second = service.subscribe(path, WatchSubscriptionKind::Git).unwrap();

        assert_ne!(first, second);
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(first).unwrap());
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(second).unwrap());
        assert_eq!(service.worktree_count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonical_state_notifies_each_subscriber_path_alias() {
        let scanner = Arc::new(EmptyScanner {
            ignored_calls: AtomicUsize::new(0),
        });
        let notifier = Arc::new(CapturingNotifier::default());
        let service = test_service_with_notifier(scanner, notifier.clone());
        let dir = tempfile::TempDir::new().unwrap();
        let alias_parent = tempfile::TempDir::new().unwrap();
        let alias = alias_parent.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();

        let original_path = dir.path().to_str().unwrap();
        let alias_path = alias.to_str().unwrap();
        service
            .subscribe(original_path, WatchSubscriptionKind::File)
            .unwrap();
        service
            .subscribe(alias_path, WatchSubscriptionKind::Git)
            .unwrap();
        let state = service.ensure_watching(original_path).unwrap();
        notifier.take();

        state.invalidate(InvalidateReason::git(false));

        for _ in 0..100 {
            let notifications = notifier.take();
            if let Some(committed) = notifications.iter().find(|n| {
                n.phase == super::super::worktree::SnapshotNotificationPhase::SnapshotCommitted
            }) {
                assert!(committed
                    .worktree_paths
                    .iter()
                    .any(|path| path == original_path));
                assert!(committed
                    .worktree_paths
                    .iter()
                    .any(|path| path == alias_path));
                assert_eq!(committed.file_watcher_ids.len(), 1);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("timed out waiting for alias notification");
    }

    #[tokio::test]
    async fn multiple_worktrees_are_independent_entries() {
        let service = test_service(Arc::new(EmptyScanner {
            ignored_calls: AtomicUsize::new(0),
        }));

        let first = service.ensure_for_tests("/repo-one");
        let second = service.ensure_for_tests("/repo-two");

        assert_eq!(service.worktree_count(), 2);
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn ignored_status_is_opt_in_and_not_cached_in_default_snapshot() {
        let scanner = Arc::new(EmptyScanner {
            ignored_calls: AtomicUsize::new(0),
        });
        let service = test_service(scanner.clone());
        let state = service.ensure_for_tests("/repo");
        state.invalidate(InvalidateReason::initial());

        for _ in 0..100 {
            if state.snapshot_for_read().version >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(state.snapshot_for_read().status.is_empty());
        let ignored = service.get_status("/repo", true).unwrap();
        assert_eq!(ignored[0].worktree_status, "ignored");
        assert_eq!(scanner.ignored_calls.load(Ordering::SeqCst), 1);
    }
}
