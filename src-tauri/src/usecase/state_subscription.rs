pub(crate) mod error;
pub(crate) mod reads;
pub(crate) use reads::StateSubscriptionRead;
pub(crate) use reads::{StateReadError, StateReadFailure, WorkspaceStateReads};
pub(crate) mod target;
pub(crate) mod value;
pub(crate) use error::SubscriptionError;
use parking_lot::Mutex;
use std::sync::Arc;
pub(crate) use target::{StateChangeSource, SubscriptionTarget, WatchRequirement};
pub(crate) use value::StateValue;

pub trait StateSubscriptionOutput: Send + Sync {
    #[cfg(any(test, feature = "test-support"))]
    fn as_any(&self) -> &dyn std::any::Any;
    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError>;
    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError>;
    fn publish(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError>;
}

pub trait StateSubscriptionDelivery: Send + Sync {
    fn start(&self) -> Result<Option<usize>, StateReadError>;
    fn claim(&self) -> bool;
    fn finish(
        &self,
        active: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError>;
}

pub(crate) fn stop_delivery(
    delivery: &dyn StateSubscriptionDelivery,
    stop: impl FnOnce() -> Result<(), SubscriptionError>,
    finish: impl FnOnce(
        &dyn Fn(&std::collections::HashSet<SubscriptionTarget>) -> Result<(), SubscriptionError>,
    ) -> Result<(), SubscriptionError>,
) -> Result<(), SubscriptionError> {
    if !delivery.claim() {
        return Ok(());
    }
    let result = stop();
    finish(&|active| delivery.finish(active))?;
    match result {
        Err(SubscriptionError::StreamEnded) => Ok(()),
        other => other,
    }
}

pub(crate) type StateSubscriptionOutputRef = Arc<dyn StateSubscriptionOutput>;

#[derive(Clone)]
pub struct StateChange {
    source: StateChangeSource,
    skip: Option<SubscriptionTarget>,
}

pub struct PendingChange {
    completed: std::sync::mpsc::Sender<()>,
}

pub struct ReadWorker {
    pub usecase: StateSubscriptionUsecase,
    pub target: SubscriptionTarget,
    pub changes: tokio::sync::broadcast::Receiver<StateChange>,
    pub waiting_changes: tokio::sync::mpsc::UnboundedReceiver<PendingChange>,
    pub cancelled: tokio::sync::oneshot::Receiver<()>,
}

pub(crate) enum ReadSignal {
    Change(StateChange),
    Waiting(PendingChange),
    Periodic,
    Lagged,
}

struct ReadStartPermit<'a> {
    usecase: &'a StateSubscriptionUsecase,
    target: &'a SubscriptionTarget,
}

impl Drop for ReadStartPermit<'_> {
    fn drop(&mut self) {
        self.usecase.starting_target.lock().take();
        self.usecase.release_inactive(self.target);
    }
}

#[derive(Clone)]
pub struct StateSubscriptionUsecase {
    publisher: StateSubscriptionOutputRef,
    changes: tokio::sync::broadcast::Sender<StateChange>,
    waiting_workers: Arc<
        Mutex<
            std::collections::HashMap<
                SubscriptionTarget,
                tokio::sync::mpsc::UnboundedSender<PendingChange>,
            >,
        >,
    >,
    #[cfg(any(test, feature = "test-support"))]
    test_changes: tokio::sync::broadcast::Sender<StateChangeSource>,
    clients: Arc<
        Mutex<
            std::collections::HashMap<String, std::collections::HashMap<SubscriptionTarget, usize>>,
        >,
    >,
    driver: tokio::sync::mpsc::UnboundedSender<ReadWorker>,
    history_paths: Vec<String>,
    hook_health_markers: String,
    pub(crate) reads: Option<Arc<dyn StateSubscriptionRead>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_repository_paths: Option<Arc<parking_lot::RwLock<Vec<String>>>>,
    watchers: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
    workers:
        Arc<Mutex<std::collections::HashMap<SubscriptionTarget, tokio::sync::oneshot::Sender<()>>>>,
    starts: Arc<tokio::sync::Mutex<()>>,
    starting_target: Arc<Mutex<Option<SubscriptionTarget>>>,
    watches: Arc<Mutex<std::collections::HashMap<WatchRequirement, u64>>>,
    pending_watch_stops: Arc<Mutex<Vec<u64>>>,
    watch_failures: Arc<
        Mutex<std::collections::HashMap<WatchRequirement, crate::domain::failure::WorkFailure>>,
    >,
}

impl StateSubscriptionUsecase {
    pub fn new_with_output(
        publisher: StateSubscriptionOutputRef,
        driver: tokio::sync::mpsc::UnboundedSender<ReadWorker>,
    ) -> Self {
        Self {
            publisher,
            changes: tokio::sync::broadcast::channel(64).0,
            waiting_workers: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            test_changes: tokio::sync::broadcast::channel(64).0,
            clients: Default::default(),
            driver,
            reads: None,
            #[cfg(any(test, feature = "test-support"))]
            test_repository_paths: None,
            history_paths: vec![],
            hook_health_markers: String::new(),
            watchers: None,
            workers: Default::default(),
            watches: Default::default(),
            pending_watch_stops: Default::default(),
            watch_failures: Default::default(),
            starts: Default::default(),
            starting_target: Default::default(),
        }
    }

    pub fn with_reads(
        mut self,
        reads: Arc<dyn StateSubscriptionRead>,
        watcher: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
        history_paths: Vec<String>,
        hook_health_markers: String,
    ) -> Self {
        self.history_paths = history_paths;
        self.hook_health_markers = hook_health_markers;
        self.reads = Some(reads);
        self.watchers = watcher;
        self
    }

    pub async fn start_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        delivery: &dyn StateSubscriptionDelivery,
    ) -> Result<(), StateReadError> {
        let _start = self.starts.lock().await;
        self.start_read_locked(client, target).await?;
        if let Err(error) = delivery.start() {
            let _ = self.stop(client, target);
            return Err(error);
        }
        Ok(())
    }

    pub async fn stop_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        delivery: &dyn StateSubscriptionDelivery,
    ) -> Result<(), SubscriptionError> {
        let _start = self.starts.lock().await;
        stop_delivery(
            delivery,
            || self.stop(client, target),
            |finish| self.with_active_targets(finish),
        )
    }

    async fn start_read_locked(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        let convert = StateReadError::from_error;
        let reads = self
            .reads
            .clone()
            .ok_or_else(|| convert(SubscriptionError::UnknownTarget))?;
        if self.join_active(client, target).map_err(convert)? {
            return Ok(());
        }
        *self.starting_target.lock() = Some(target.clone());
        let _permit = ReadStartPermit {
            usecase: self,
            target,
        };
        reads.acquire_external(target);
        let changes = self.changes.subscribe();
        let value = match reads.refresh_external(target).await {
            Ok(()) => reads.read(target).await,
            Err(error) => Err(error),
        };
        if let Err(error) = self.start(client, target) {
            return Err(convert(error));
        }
        let failures = self.reconcile_watches();
        self.apply_watch_failures(target, failures).await;
        if !self.clients.lock().contains_key(client) {
            return Err(convert(SubscriptionError::StreamEnded));
        }
        if let Err(error) = self.publish_read(target, value, true) {
            let _ = self.stop(client, target);
            return Err(convert(error));
        }
        let mut workers = self.workers.lock();
        if workers.contains_key(target) {
            return Ok(());
        }
        let (waiting_sender, waiting_changes) = tokio::sync::mpsc::unbounded_channel();
        self.waiting_workers
            .lock()
            .insert(target.clone(), waiting_sender);
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        workers.insert(target.clone(), cancel);
        if self
            .driver
            .send(ReadWorker {
                usecase: self.clone(),
                target: target.clone(),
                changes,
                waiting_changes,
                cancelled,
            })
            .is_err()
        {
            workers.remove(target);
            self.waiting_workers.lock().remove(target);
            drop(workers);
            let _ = self.stop(client, target);
            return Err(convert(SubscriptionError::StreamEnded));
        }
        Ok(())
    }

    pub(crate) async fn refresh_read(
        &self,
        target: &SubscriptionTarget,
        signal: ReadSignal,
        changes: &mut tokio::sync::broadcast::Receiver<StateChange>,
        waiting: &mut tokio::sync::mpsc::UnboundedReceiver<PendingChange>,
    ) {
        if !self.is_active(target) {
            return;
        }
        let mut refresh_external = false;
        let mut completed = Vec::new();
        match signal {
            ReadSignal::Change(change) => {
                if change.skip.as_ref() == Some(target) || !target.affected_by(&change.source) {
                    return;
                }
                refresh_external = adds_external_information(target, &change.source);
            }
            ReadSignal::Waiting(change) => completed.push(change.completed),
            ReadSignal::Periodic => {
                let requirements = self.required_watches(target, None);
                let watch_failed = requirements
                    .iter()
                    .any(|requirement| self.watch_failures.lock().contains_key(requirement));
                if !target.external_information() && !watch_failed {
                    return;
                }
                refresh_external = target.external_information();
            }
            ReadSignal::Lagged => {}
        }
        loop {
            use tokio::sync::broadcast::error::TryRecvError;
            match changes.try_recv() {
                Ok(change) => refresh_external |= adds_external_information(target, &change.source),
                Err(TryRecvError::Lagged(_)) => continue,
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        while let Ok(change) = waiting.try_recv() {
            completed.push(change.completed);
        }
        let reads = self.reads.as_ref().expect("subscription reads");
        let refresh = if refresh_external {
            reads.refresh_external(target).await
        } else {
            Ok(())
        };
        let failures = self.reconcile_watches();
        let result = match refresh {
            Ok(()) => reads.read(target).await,
            Err(error) => Err(error),
        };
        self.apply_watch_failures(target, failures).await;
        let publication = self.publish_read(target, result, false);
        if let Err(error) = publication {
            log::error!("State publication failed: {error}");
        }
        for completed in completed {
            let _ = completed.send(());
        }
    }

    fn release_inactive(&self, target: &SubscriptionTarget) {
        let starting = self.starting_target.lock();
        if starting.as_ref() == Some(target) {
            return;
        }
        let clients = self.clients.lock();
        if !clients.values().any(|targets| targets.contains_key(target)) {
            if let Some(reads) = &self.reads {
                reads.release_external(target);
            }
        }
    }

    fn required_watches(
        &self,
        target: &SubscriptionTarget,
        repositories: Option<&[String]>,
    ) -> Vec<WatchRequirement> {
        let reads = self.reads.as_ref().expect("subscription reads");
        let paths;
        let repositories = match repositories {
            Some(paths) => paths,
            None => {
                paths = reads.repositories();
                &paths
            }
        };
        target.watches(
            repositories,
            &self.history_paths,
            &reads.review_comments_dir(),
            &reads.workflows_dir(),
            &self.hook_health_markers,
        )
    }

    fn reconcile_watches(&self) -> Vec<(WatchRequirement, StateReadError)> {
        let mut failures = Vec::new();
        if let (Some(reads), Some(watcher)) = (&self.reads, &self.watchers) {
            self.pending_watch_stops
                .lock()
                .retain(|id| match watcher.stop(*id) {
                    Ok(()) => false,
                    Err(error) => {
                        log::error!("State watch cleanup failed: {error}");
                        true
                    }
                });
            let mut watches = self.watches.lock();
            let (repositories, path_failures) = reads.watch_paths();
            if self.is_active(&SubscriptionTarget::Workspaces) {
                failures.extend(path_failures.into_iter().map(|(path, failure)| {
                    (
                        WatchRequirement::Git(path),
                        StateReadError::from_error(
                            crate::usecase::repository_state::RepositoryStateError::Background {
                                kind: failure.kind,
                                message: failure.message,
                            },
                        ),
                    )
                }));
            }
            let required: std::collections::HashSet<_> = self
                .active_targets()
                .into_iter()
                .flat_map(|target| self.required_watches(&target, Some(&repositories)))
                .collect();
            let current: std::collections::HashSet<_> = watches.keys().cloned().collect();
            let start: Vec<_> = required.difference(&current).cloned().collect();
            let stop: Vec<_> = current.difference(&required).cloned().collect();
            for requirement in stop {
                if let Some(id) = watches.get(&requirement).copied() {
                    match watcher.stop(id) {
                        Ok(()) => {
                            watches.remove(&requirement);
                        }
                        Err(error) => log::error!("State watch cleanup failed: {error}"),
                    }
                }
            }
            for requirement in start {
                let watch_id = Arc::new(std::sync::atomic::AtomicU64::new(0));
                let result = match &requirement {
                    crate::usecase::state_subscription::WatchRequirement::Git(path) => {
                        watcher.start_git_dir(path)
                    }
                    crate::usecase::state_subscription::WatchRequirement::Files(path, source) => {
                        let subscriptions = self.clone();
                        let source = source.clone();
                        let watched = requirement.clone();
                        let watch_id = watch_id.clone();
                        watcher.start_files(
                            path,
                            Arc::new(move |event| match event {
                                Ok(()) => subscriptions.notify(source.clone()),
                                Err(error) => {
                                    subscriptions.file_watch_failed(&watched, &watch_id, error)
                                }
                            }),
                        )
                    }
                };
                match result {
                    Ok(id) => {
                        watch_id.store(id, std::sync::atomic::Ordering::Release);
                        watches.insert(requirement, id);
                    }
                    Err(error) => {
                        failures.push((requirement, StateReadError::from_error(error)));
                    }
                }
            }
        }
        let mut workers = self.workers.lock();
        let active = self.active_targets();
        workers.retain(|target, _cancel| {
            if active.contains(target) {
                true
            } else {
                self.waiting_workers.lock().remove(target);

                self.release_inactive(target);
                false
            }
        });
        failures
    }

    fn file_watch_failed(
        &self,
        requirement: &WatchRequirement,
        watch_id: &std::sync::atomic::AtomicU64,
        message: String,
    ) {
        let id = {
            let mut watches = self.watches.lock();
            let id = watch_id.load(std::sync::atomic::Ordering::Acquire);
            if watches.get(requirement) != Some(&id) {
                return;
            }
            watches.remove(requirement);
            let error = crate::usecase::watcher::UsecaseError::File(message);
            let mut failures = self.watch_failures.lock();
            failures.insert(
                requirement.clone(),
                crate::domain::failure::WorkFailure::from_error(&error),
            );
            for target in self.active_targets() {
                let requirements = self.required_watches(&target, None);
                if !requirements.contains(requirement) {
                    continue;
                }
                if let Some(error) = Self::watch_failure(&requirements, &failures, false) {
                    if let Err(error) = self.publisher.publish_failure(&target, error) {
                        log::error!("State watch publication failed: {error}");
                    }
                }
            }
            id
        };
        if let Some(watcher) = &self.watchers {
            if let Err(error) = watcher.stop(id) {
                log::error!("State watch cleanup failed: {error}");
                self.pending_watch_stops.lock().push(id);
            }
        }
    }

    fn report_watch_failures(&self, failures: Vec<(WatchRequirement, StateReadError)>) {
        for (requirement, error) in failures {
            log::error!("State watch failed for {requirement:?}: {error}");
        }
    }

    async fn apply_watch_failures(
        &self,
        target: &SubscriptionTarget,
        failures: Vec<(WatchRequirement, StateReadError)>,
    ) {
        let reads = self.reads.as_ref().expect("subscription reads");
        let required: std::collections::HashSet<_> = self
            .active_targets()
            .into_iter()
            .flat_map(|target| self.required_watches(&target, None))
            .collect();
        let changed = {
            let watches = self.watches.lock();
            let mut previous = self.watch_failures.lock();
            let mut next = std::collections::HashMap::new();
            for (requirement, error) in failures {
                if self.watchers.is_some()
                    && matches!(&requirement, WatchRequirement::Files(..))
                    && watches.contains_key(&requirement)
                {
                    continue;
                }
                let failure = crate::domain::failure::WorkFailure::from_error(&error);
                next.insert(requirement, failure);
            }
            if self.watchers.is_some() {
                for (requirement, failure) in previous.iter() {
                    if matches!(requirement, WatchRequirement::Files(..))
                        && required.contains(requirement)
                        && !watches.contains_key(requirement)
                    {
                        next.entry(requirement.clone())
                            .or_insert_with(|| failure.clone());
                    }
                }
            }
            let changed: std::collections::HashSet<_> = previous
                .keys()
                .chain(next.keys())
                .filter(|key| previous.get(*key) != next.get(*key))
                .cloned()
                .collect();
            *previous = next;
            changed
        };
        for affected in self.active_targets() {
            if affected == *target {
                continue;
            }
            let requirements = self.required_watches(&affected, None);
            if !changed.iter().any(|requirement| {
                requirements.contains(requirement)
                    || (affected == SubscriptionTarget::Workspaces
                        && matches!(requirement, WatchRequirement::Git(_)))
            }) {
                continue;
            }
            let value = reads.read(&affected).await;
            let publication = self.publish_read(&affected, value, false);
            if let Err(error) = publication {
                log::error!("State watch publication failed: {error}");
            }
        }
    }

    fn publish_read(
        &self,
        target: &SubscriptionTarget,
        result: Result<StateValue, StateReadError>,
        initial: bool,
    ) -> Result<(), SubscriptionError> {
        let requirements = self.required_watches(target, None);
        let failures = self.watch_failures.lock();
        let result = Self::with_watch_failures(result, &requirements, &failures);
        match result {
            Ok(value) if initial => self.publisher.publish_initial(target, value),
            Ok(value) => self.publisher.publish(target, value, None),
            Err(error) => self.publisher.publish_failure(target, error),
        }
    }

    fn with_watch_failures(
        mut result: Result<StateValue, StateReadError>,
        requirements: &[WatchRequirement],
        failures: &std::collections::HashMap<WatchRequirement, crate::domain::failure::WorkFailure>,
    ) -> Result<StateValue, StateReadError> {
        for (requirement, failure) in failures {
            if let (WatchRequirement::Git(path), Ok(StateValue::Workspaces(list))) =
                (requirement, &mut result)
            {
                for repository in &mut list.repositories {
                    if repository.path == *path {
                        repository.worktrees.error = Some(failure.clone());
                    }
                    if let Some(worktrees) = &mut repository.worktrees.value {
                        for worktree in worktrees {
                            if worktree.worktree.path == *path {
                                worktree.tree.error = Some(failure.clone());
                                worktree.dirty_count.error = Some(failure.clone());
                            }
                        }
                    }
                }
            }
        }
        let workspaces = matches!(&result, Ok(StateValue::Workspaces(_)));
        match Self::watch_failure(requirements, failures, workspaces) {
            Some(error) => Err(error),
            None => result,
        }
    }

    fn watch_failure(
        requirements: &[WatchRequirement],
        failures: &std::collections::HashMap<WatchRequirement, crate::domain::failure::WorkFailure>,
        workspaces: bool,
    ) -> Option<StateReadError> {
        failures.iter().find_map(|(requirement, failure)| {
            if (workspaces && matches!(requirement, WatchRequirement::Git(_)))
                || !requirements.contains(requirement)
            {
                return None;
            }
            Some(StateReadError::from_error(
                crate::usecase::repository_state::RepositoryStateError::Background {
                    kind: failure.kind,
                    message: failure.message.clone(),
                },
            ))
        })
    }

    #[cfg(test)]
    pub fn publisher(&self) -> StateSubscriptionOutputRef {
        self.publisher.clone()
    }

    pub fn notify(&self, source: StateChangeSource) {
        self.send_change(source, None);
    }

    pub(crate) fn notify_and_wait(&self, source: StateChangeSource, target: &SubscriptionTarget) {
        let worker = self.waiting_workers.lock().get(target).cloned();
        if let Some(worker) = worker {
            let (completed, receiver) = std::sync::mpsc::channel();
            self.send_change(source, Some(target.clone()));
            if worker.send(PendingChange { completed }).is_ok() {
                let _ = receiver.recv();
            }
        } else {
            self.send_change(source, None);
        }
    }

    fn send_change(&self, source: StateChangeSource, skip: Option<SubscriptionTarget>) {
        #[cfg(any(test, feature = "test-support"))]
        let _ = self.test_changes.send(source.clone());
        let _ = self.changes.send(StateChange { source, skip });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn changes(&self) -> tokio::sync::broadcast::Receiver<StateChangeSource> {
        self.test_changes.subscribe()
    }

    pub fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        let subscriptions = clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        *subscriptions.entry(target.clone()).or_default() += 1;
        Ok(())
    }

    pub fn stop(&self, client: &str, target: &SubscriptionTarget) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        let subscriptions = clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        if let Some(count) = subscriptions.get_mut(target) {
            *count -= 1;
            if *count > 0 {
                return Ok(());
            }
            subscriptions.remove(target);
        }
        drop(clients);
        self.report_watch_failures(self.reconcile_watches());
        Ok(())
    }

    pub fn open_client(&self, id: String) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        if clients.contains_key(&id) {
            return Err(SubscriptionError::AlreadyExists);
        }
        clients.insert(id, Default::default());
        Ok(())
    }

    pub fn close_client(&self, id: &str) {
        self.clients.lock().remove(id);
        self.report_watch_failures(self.reconcile_watches());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn output_ref(&self) -> &dyn StateSubscriptionOutput {
        self.publisher.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_worker_count(&self) -> usize {
        self.workers.lock().len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_watches(&self) -> std::collections::HashMap<WatchRequirement, u64> {
        self.watches.lock().clone()
    }

    fn is_active(&self, target: &SubscriptionTarget) -> bool {
        self.clients
            .lock()
            .values()
            .any(|targets| targets.contains_key(target))
    }

    pub fn active_targets(&self) -> std::collections::HashSet<SubscriptionTarget> {
        self.with_active_targets(Clone::clone)
    }

    /// 購読中の対象を読む間、購読者の登録を止める。出版側の保持を捨てる判断はこの中で行い、
    /// 判断の後に登録された購読者が捨てられた対象を有効と見なさないようにする。
    pub(crate) fn with_active_targets<R>(
        &self,
        read: impl FnOnce(&std::collections::HashSet<SubscriptionTarget>) -> R,
    ) -> R {
        let clients = self.clients.lock();
        let active = clients
            .values()
            .flat_map(|targets| targets.keys().cloned())
            .collect();
        read(&active)
    }

    /// 他の購読者が既に購読中の対象なら、同じロックの中で `client` を登録して true を返す。
    fn join_active(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<bool, SubscriptionError> {
        let mut clients = self.clients.lock();
        if !clients.values().any(|targets| targets.contains_key(target)) {
            return Ok(false);
        }
        clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?
            .entry(target.clone())
            .and_modify(|count| *count += 1)
            .or_insert(1);
        Ok(true)
    }
}

/// Repository の増減の影響を受ける対象と、その Repository の Notion 設定が変わった対象の外部情報を取り直す。
fn adds_external_information(target: &SubscriptionTarget, source: &StateChangeSource) -> bool {
    target.affected_by(source)
        && ((*source == StateChangeSource::Repositories && target.external_information())
            || matches!(source, StateChangeSource::NotionConfig(_)))
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;

#[cfg(feature = "test-support")]
impl StateSubscriptionUsecase {
    pub fn test_remove_client_registration(&self, client_id: &str) {
        self.clients.lock().remove(client_id);
    }
}
