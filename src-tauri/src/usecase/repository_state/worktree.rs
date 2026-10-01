use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::error::RepositoryStateError;
use super::runtime::{RepositoryStateInvalidationSender, RepositoryStateWorkerRuntime, ScanWorker};
use super::scanner::RepositoryScanner;
use super::snapshot::{RepositorySnapshot, RepositorySnapshotParts};
use super::worker::InvalidateReason;
use crate::domain::repository::Worktree;
use crate::usecase::fetched::Fetched;

pub trait RepositoryStateWatchSession: Send + Sync {}

impl<T> RepositoryStateWatchSession for T where T: Send + Sync {}

pub trait RepositoryStateWatcher: Send + Sync {
    fn next_watcher_id(&self) -> u64;

    fn start_watchers(
        &self,
        state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError>;
}

#[cfg(test)]
#[derive(Default)]
pub struct NoopRepositoryStateWatcher;

#[cfg(test)]
impl RepositoryStateWatcher for NoopRepositoryStateWatcher {
    fn next_watcher_id(&self) -> u64 {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }

    fn start_watchers(
        &self,
        _state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError> {
        Ok(Box::new(()))
    }
}

pub struct WorktreeState {
    worktree_path: String,
    /// この path が Repository の root（main worktree）か。root だけが worktree の並びを持つ。
    is_repository_root: bool,
    pub(crate) scan_lock: tokio::sync::Mutex<()>,
    snapshot: RwLock<Arc<RepositorySnapshot>>,
    worktrees: RwLock<Fetched<Vec<Worktree>>>,
    scan_failure: RwLock<Option<crate::usecase::failure::WorkFailure>>,
    version: AtomicU64,
    requested_generation: AtomicU64,
    applied_generation: AtomicU64,
    refreshing: AtomicBool,
    shutdown: AtomicBool,
    invalidate_tx: Box<dyn RepositoryStateInvalidationSender>,
    watchers: Mutex<Option<Box<dyn RepositoryStateWatchSession>>>,
    subscriptions: Mutex<HashMap<u64, String>>,
    state_subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
}

impl WorktreeState {
    pub fn new(
        worktree_path: String,
        is_repository_root: bool,
        scanner: Arc<dyn RepositoryScanner>,
        state_subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        debounce: Duration,
    ) -> Arc<Self> {
        let (invalidate_tx, invalidate_rx) = runtime.invalidation_channel();
        let state = Arc::new(Self {
            worktree_path,
            is_repository_root,
            scan_lock: tokio::sync::Mutex::new(()),
            snapshot: RwLock::new(Arc::new(RepositorySnapshot::loading())),
            worktrees: RwLock::new(Fetched::default()),
            scan_failure: RwLock::new(None),
            version: AtomicU64::new(0),
            requested_generation: AtomicU64::new(0),
            applied_generation: AtomicU64::new(0),
            refreshing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            invalidate_tx,
            watchers: Mutex::new(None),
            subscriptions: Mutex::new(HashMap::new()),
            state_subscriptions,
        });
        runtime.spawn_worker(ScanWorker {
            state: state.clone(),
            scanner,
            receiver: invalidate_rx,
            debounce,
        });
        state
    }

    pub async fn scan_once(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        runtime: &dyn RepositoryStateWorkerRuntime,
    ) -> Result<Option<Arc<RepositorySnapshot>>, RepositoryStateError> {
        let _scan = self.scan_lock.lock().await;
        if self.is_shutdown() {
            return Ok(None);
        }
        self.set_refreshing(true);
        let generation = self.requested_generation();
        let parts = runtime.scan(scanner, self.worktree_path.clone()).await?;
        Ok(self.commit_snapshot(parts, generation))
    }

    /// Repository の root なら worktree の並びを読み直す。失敗しても最後に読めた並びを残す。
    pub async fn scan_worktrees_once(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        runtime: &dyn RepositoryStateWorkerRuntime,
    ) {
        if !self.is_repository_root || self.is_shutdown() {
            return;
        }
        let result = runtime
            .scan_worktrees(scanner.clone(), self.worktree_path.clone())
            .await;
        if result.is_ok() {
            if let Err(err) = scanner.prune_stale_branch_bases(&self.worktree_path) {
                log::warn!(
                    "repository branch base GC failed for {}: {err}",
                    self.worktree_path
                );
            }
        }
        self.worktrees.write().record(
            result.map_err(|error| crate::domain::failure::WorkFailure::from_error(&error)),
        );
    }

    /// 走査の結果を確定して知らせる。変更の状態を走査していないときは `status` は None。
    pub fn finish_scan(
        &self,
        status: Option<Result<Option<Arc<RepositorySnapshot>>, RepositoryStateError>>,
        reason: InvalidateReason,
    ) -> Option<InvalidateReason> {
        self.set_refreshing(false);
        match status {
            Some(Ok(None)) => Some(reason),
            Some(Ok(Some(_))) | None => {
                self.notify_snapshot_changed();
                None
            }
            Some(Err(err)) => {
                log::warn!(
                    "repository snapshot scan failed for {}: {err}",
                    self.worktree_path
                );
                self.mark_scan_failed(&err);
                self.notify_snapshot_changed();
                None
            }
        }
    }

    pub fn is_repository_root(&self) -> bool {
        self.is_repository_root
    }

    /// Repository の worktree の並び。root でなければ、まだ読めていないのと同じ。
    pub fn worktrees(&self) -> Fetched<Vec<Worktree>> {
        self.worktrees.read().clone()
    }

    pub fn worktree_path(&self) -> &str {
        &self.worktree_path
    }

    pub fn requested_generation(&self) -> u64 {
        self.requested_generation.load(Ordering::SeqCst)
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn set_refreshing(&self, refreshing: bool) {
        self.refreshing.store(refreshing, Ordering::SeqCst);
    }

    pub fn add_subscription(&self, id: u64, worktree_path: String) {
        self.subscriptions.lock().insert(id, worktree_path);
    }

    pub fn release_subscription(&self, id: u64) -> bool {
        self.subscriptions.lock().remove(&id).is_some()
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscriptions.lock().len()
    }

    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        self.watchers.lock().take();
        let _ = self.invalidate_tx.send(InvalidateReason::shutdown());
    }

    pub fn start_watchers(
        self: &Arc<Self>,
        watcher: &dyn RepositoryStateWatcher,
    ) -> Result<(), RepositoryStateError> {
        let mut watchers = self.watchers.lock();
        if watchers.is_some() {
            return Ok(());
        }

        *watchers = Some(watcher.start_watchers(self.clone())?);
        drop(watchers);

        self.invalidate(InvalidateReason::change());
        Ok(())
    }

    pub fn invalidate(&self, reason: InvalidateReason) {
        if self.is_shutdown() {
            return;
        }
        {
            let _snapshot = self.snapshot.write();
            self.requested_generation.fetch_add(1, Ordering::SeqCst);
        }
        if self.invalidate_tx.send(reason).is_err() {
            log::warn!(
                "repository snapshot worker is stopped for {}",
                self.worktree_path
            );
        }
    }

    pub fn read_snapshot(&self) -> Result<Arc<RepositorySnapshot>, RepositoryStateError> {
        let snapshot = self.snapshot_for_read();
        if let Some(error) = self.scan_failure.read().as_ref() {
            return Err(RepositoryStateError::Background {
                kind: error.kind,
                message: error.message.clone(),
            });
        }
        Ok(snapshot)
    }

    pub fn dirty_count(&self) -> Fetched<usize> {
        let snapshot = self.snapshot.read();
        let error = self.scan_failure.read();
        Fetched {
            value: (snapshot.version > 0).then_some(snapshot.dirty_count),
            error: error.clone(),
        }
    }

    pub fn snapshot_for_read(&self) -> Arc<RepositorySnapshot> {
        let snapshot = self.snapshot.read().clone();
        if self.refreshing.load(Ordering::SeqCst) {
            let stale = snapshot.version > 0;
            return Arc::new(snapshot.with_read_flags(stale, true));
        }
        snapshot
    }

    pub(crate) fn commit_snapshot(
        &self,
        parts: RepositorySnapshotParts,
        generation: u64,
    ) -> Option<Arc<RepositorySnapshot>> {
        let mut current = self.snapshot.write();
        if self.requested_generation() != generation {
            return None;
        }
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(parts.into_snapshot(version));
        *current = snapshot.clone();
        *self.scan_failure.write() = None;
        self.applied_generation.store(generation, Ordering::SeqCst);
        Some(snapshot)
    }

    pub(crate) fn notify_snapshot_changed(&self) {
        self.state_subscriptions.notify(
            crate::usecase::state_subscription::StateChangeSource::Repository(
                self.notification_paths(),
            ),
        );
    }

    pub(crate) fn mark_scan_failed(&self, error: &RepositoryStateError) {
        *self.scan_failure.write() = Some(crate::usecase::failure::WorkFailure::from_error(error));
        self.refreshing.store(false, Ordering::SeqCst);
        let current = self.snapshot.read().clone();
        if current.version == 0 && current.flags.loading {
            *self.snapshot.write() = Arc::new(current.with_read_flags(false, false));
        }
    }

    fn notification_paths(&self) -> Vec<String> {
        let subscriptions = self.subscriptions.lock();
        let mut paths = Vec::new();
        for path in subscriptions.values() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        if paths.is_empty() {
            paths.push(self.worktree_path.clone());
        }
        paths
    }
}

impl Drop for WorktreeState {
    fn drop(&mut self) {
        self.watchers.lock().take();
    }
}

#[cfg(test)]
#[path = "worktree_test.rs"]
mod worktree_tests;
