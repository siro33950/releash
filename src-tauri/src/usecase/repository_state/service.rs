use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::domain::repository::Worktree;
use crate::usecase::fetched::Fetched;

use super::error::RepositoryStateError;
use super::runtime::{RepositoryStateWorkerRuntime, WorktreePathNormalizer};
use super::scanner::RepositoryScanner;
use super::snapshot::RepositorySnapshot;
use super::worktree::{RepositoryStateWatcher, WorktreeState};

pub trait RepositoryStateRepository: Send + Sync {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError>;
}

pub struct RepositoryStateService {
    repository: Arc<dyn RepositoryStateRepository>,
    scanner: Arc<dyn RepositoryScanner>,
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    watcher: Arc<dyn RepositoryStateWatcher>,
    runtime: Arc<dyn RepositoryStateWorkerRuntime>,
    path_normalizer: Arc<dyn WorktreePathNormalizer>,
    workers: tokio::sync::mpsc::UnboundedSender<super::runtime::ScanWorker>,
    worktrees: RwLock<HashMap<PathBuf, Arc<WorktreeState>>>,
    /// path から解決した Repository の root。root は変わらないので、解決できたら持ち続ける。
    roots: RwLock<HashMap<String, String>>,
}

impl RepositoryStateService {
    pub fn new(
        repository: Arc<dyn RepositoryStateRepository>,
        scanner: Arc<dyn RepositoryScanner>,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
        watcher: Arc<dyn RepositoryStateWatcher>,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        path_normalizer: Arc<dyn WorktreePathNormalizer>,
        workers: tokio::sync::mpsc::UnboundedSender<super::runtime::ScanWorker>,
    ) -> Self {
        Self {
            workers,
            repository,
            scanner,
            subscriptions,
            watcher,
            runtime,
            path_normalizer,
            worktrees: RwLock::new(HashMap::new()),
            roots: RwLock::new(HashMap::new()),
        }
    }

    pub fn start_git_dir_watching(&self, repo_path: &str) -> Result<u64, RepositoryStateError> {
        self.subscribe(repo_path)
    }

    pub fn get_snapshot(
        &self,
        worktree_path: &str,
    ) -> Result<Arc<RepositorySnapshot>, RepositoryStateError> {
        let key = self.canonical_worktree_key(worktree_path)?;
        if let Some(existing) = self.worktrees.read().get(&key) {
            return existing.read_snapshot();
        }

        let canonical_path = key.to_string_lossy().to_string();
        let snapshot = self.scanner.scan(&canonical_path)?.into_snapshot(0);
        Ok(Arc::new(snapshot))
    }

    /// path が属する Repository の root（main worktree の場所）。
    pub fn repository_root(&self, path: &str) -> Result<String, RepositoryStateError> {
        if let Some(root) = self.roots.read().get(path) {
            return Ok(root.clone());
        }
        let root = self.repository.main_repo_path(path)?;
        self.roots.write().insert(path.to_string(), root.clone());
        Ok(root)
    }

    /// Repository の worktree の並び。監視していない Repository は、まだ読めていない扱いになる。
    pub fn worktrees(&self, repo_path: &str) -> Fetched<Vec<Worktree>> {
        match self.repository_root_state(repo_path) {
            Ok(Some(state)) => state.worktrees(),
            Ok(None) => Fetched::default(),
            Err(error) => Fetched {
                value: None,
                error: Some(crate::domain::failure::WorkFailure::from_error(&error)),
            },
        }
    }

    pub fn dirty_count(&self, path: &str) -> Fetched<usize> {
        match self.canonical_worktree_key(path) {
            Ok(key) => self
                .worktrees
                .read()
                .get(&key)
                .map(|state| state.dirty_count())
                .unwrap_or_default(),
            Err(error) => Fetched {
                value: None,
                error: Some(crate::domain::failure::WorkFailure::from_error(&error)),
            },
        }
    }

    /// Repository の worktree の並びと、各 worktree の変更の状態を読み直す。
    pub async fn rescan(&self, repo_path: &str) -> Result<(), RepositoryStateError> {
        let Some(root) = self.repository_root_state(repo_path)? else {
            return Ok(());
        };
        root.scan_worktrees_once(self.scanner.clone(), self.runtime.as_ref())
            .await;
        let mut states = vec![root.clone()];
        for worktree in root.worktrees().value.unwrap_or_default() {
            let Ok(key) = self.canonical_worktree_key(&worktree.path) else {
                continue;
            };
            if let Some(state) = self.worktrees.read().get(&key) {
                if !Arc::ptr_eq(state, &root) {
                    states.push(state.clone());
                }
            }
        }
        futures_util::future::join_all(states.iter().map(|state| async move {
            if let Err(error) = self.rescan_status(state).await {
                log::warn!(
                    "repository snapshot rescan failed for {}: {error}",
                    state.worktree_path()
                );
            }
            state.notify_snapshot_changed();
        }))
        .await;
        Ok(())
    }

    pub async fn rescan_status(&self, state: &WorktreeState) -> Result<(), RepositoryStateError> {
        let _scan = state.scan_lock.lock().await;
        for _ in 0..2 {
            let generation = state.requested_generation();
            let result = self
                .runtime
                .scan(self.scanner.clone(), state.worktree_path().to_owned())
                .await;
            if state.requested_generation() != generation {
                continue;
            }
            let parts = match result {
                Ok(parts) => parts,
                Err(error) => {
                    state.mark_scan_failed(&error);
                    return Err(error);
                }
            };
            if state.commit_snapshot(parts, generation).is_some() {
                return Ok(());
            }
        }
        Err(RepositoryStateError::ScanInvalidated)
    }

    fn repository_root_state(
        &self,
        repo_path: &str,
    ) -> Result<Option<Arc<WorktreeState>>, RepositoryStateError> {
        let root = self.repository_root(repo_path)?;
        let key = self.canonical_worktree_key(&root)?;
        Ok(self
            .worktrees
            .read()
            .get(&key)
            .filter(|state| state.is_repository_root())
            .cloned())
    }

    pub fn stop_watching(&self, watcher_id: u64) -> Result<bool, RepositoryStateError> {
        Ok(self.release_watching(watcher_id))
    }

    fn release_watching(&self, watcher_id: u64) -> bool {
        let mut found = false;
        let mut removed = None;

        {
            let mut worktrees = self.worktrees.write();
            let mut empty_key = None;

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
                removed = worktrees.remove(&key);
            }
        }

        // watcher の破棄はブロックし得るため worktrees ロックの外で行う（#1641）
        if let Some(state) = removed {
            state.shutdown();
        }

        found
    }

    pub fn subscribe(&self, worktree_path: &str) -> Result<u64, RepositoryStateError> {
        let subscription_id = self.watcher.next_watcher_id();
        self.ensure_watching_with_subscription(worktree_path, Some(subscription_id))?;
        Ok(subscription_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn ensure_watching(
        &self,
        worktree_path: &str,
    ) -> Result<Arc<WorktreeState>, RepositoryStateError> {
        self.ensure_watching_with_subscription(worktree_path, None)
    }

    fn ensure_watching_with_subscription(
        &self,
        worktree_path: &str,
        subscription: Option<u64>,
    ) -> Result<Arc<WorktreeState>, RepositoryStateError> {
        let key = self.canonical_worktree_key(worktree_path)?;
        if let Some(existing) = self.worktrees.read().get(&key) {
            if let Some(id) = subscription {
                existing.add_subscription(id, worktree_path.to_string());
            }
            return Ok(existing.clone());
        }

        // watcher の生成は watch ごとに FSEvents ストリームの破棄・再作成を伴い
        // ブロックし得るため worktrees ロックの外で行う（#1641）。生成が競合した
        // 場合は先に登録された state を採用し、負けた側の watcher は破棄する。
        let canonical_path = key.to_string_lossy().to_string();
        let is_repository_root = self
            .repository_root(worktree_path)
            .ok()
            .and_then(|root| self.canonical_worktree_key(&root).ok())
            .is_some_and(|root| root == key);
        let state = WorktreeState::new(
            canonical_path,
            is_repository_root,
            self.scanner.clone(),
            self.subscriptions.clone(),
            self.runtime.clone(),
            self.workers.clone(),
        );
        if let Err(err) = state.start_watchers(self.watcher.as_ref()) {
            state.shutdown();
            return Err(err);
        }

        let mut worktrees = self.worktrees.write();
        if let Some(existing) = worktrees.get(&key) {
            if let Some(id) = subscription {
                existing.add_subscription(id, worktree_path.to_string());
            }
            let existing = existing.clone();
            drop(worktrees);
            state.shutdown();
            return Ok(existing);
        }
        if let Some(id) = subscription {
            state.add_subscription(id, worktree_path.to_string());
        }
        worktrees.insert(key, state.clone());
        Ok(state)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn ensure_for_tests(&self, worktree_path: &str) -> Arc<WorktreeState> {
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
            true,
            self.scanner.clone(),
            self.subscriptions.clone(),
            self.runtime.clone(),
            self.workers.clone(),
        );
        worktrees.insert(key, state.clone());
        state
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn worktree_count(&self) -> usize {
        self.worktrees.read().len()
    }

    fn canonical_worktree_key(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
        self.path_normalizer.normalize(worktree_path)
    }
}

#[cfg(test)]
#[path = "service_test.rs"]
mod service_tests;
