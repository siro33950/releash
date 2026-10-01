mod error;
mod reads;
pub(crate) use reads::StateSubscriptionRead;
pub(crate) use reads::{StateReadError, StateReadFailure, WorkspaceStateReads};
mod target;
mod value;
pub(crate) use error::SubscriptionError;
use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use std::sync::Arc;
pub(crate) use target::{StateChangeSource, SubscriptionTarget, WatchRequirement};
pub(crate) use value::StateValue;

pub(crate) trait SubscriptionTimer: Send + Sync {
    fn interval(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>>;
}

pub(crate) trait StateSubscriptionOutput: Send + Sync {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
    fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError>;
    fn stop(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        active: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError>;
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

pub(crate) type StateSubscriptionOutputRef = Arc<dyn StateSubscriptionOutput>;

#[derive(Clone)]
struct StateChange {
    source: StateChangeSource,
    skip: Option<SubscriptionTarget>,
}

struct PendingChange {
    completed: std::sync::mpsc::Sender<()>,
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
pub(crate) struct StateSubscriptionUsecase {
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
    #[cfg(test)]
    test_changes: tokio::sync::broadcast::Sender<StateChangeSource>,
    clients: Arc<
        Mutex<std::collections::HashMap<String, std::collections::HashSet<SubscriptionTarget>>>,
    >,
    timer: Arc<dyn SubscriptionTimer>,
    history_paths: Vec<String>,
    hook_health_markers: String,
    pub(crate) reads: Option<Arc<dyn StateSubscriptionRead>>,
    #[cfg(test)]
    pub(crate) test_repository_paths: Option<Arc<parking_lot::RwLock<Vec<String>>>>,
    watchers: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
    workers: Arc<Mutex<std::collections::HashMap<SubscriptionTarget, tokio::task::JoinHandle<()>>>>,
    starts: Arc<tokio::sync::Mutex<()>>,
    starting_target: Arc<Mutex<Option<SubscriptionTarget>>>,
    watches: Arc<Mutex<std::collections::HashMap<WatchRequirement, u64>>>,
}

impl StateSubscriptionUsecase {
    pub(crate) async fn start_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        self.start_read(client, target).await?;
        if let Err(error) = self.publisher.start(client, target, cursor) {
            let _ = self.stop(client, target);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) async fn stop_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        self.stop_read(client, target).await?;
        self.with_active_targets(|active| self.publisher.stop(client, target, active))
    }

    pub fn new_with_output(
        publisher: StateSubscriptionOutputRef,
        timer: Arc<dyn SubscriptionTimer>,
    ) -> Self {
        Self {
            publisher,
            changes: tokio::sync::broadcast::channel(64).0,
            waiting_workers: Default::default(),
            #[cfg(test)]
            test_changes: tokio::sync::broadcast::channel(64).0,
            clients: Default::default(),
            timer,
            reads: None,
            #[cfg(test)]
            test_repository_paths: None,
            history_paths: vec![],
            hook_health_markers: String::new(),
            watchers: None,
            workers: Default::default(),
            watches: Default::default(),
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

    pub async fn start_read(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        let convert = StateReadError::from_error;
        let reads = self
            .reads
            .clone()
            .ok_or_else(|| convert(SubscriptionError::UnknownTarget))?;
        // ponytail: subscription starts are serialized; split by target if initial reads contend.
        let _start = self.starts.lock().await;
        if self.join_active(client, target).map_err(convert)? {
            return Ok(());
        }
        *self.starting_target.lock() = Some(target.clone());
        let _permit = ReadStartPermit {
            usecase: self,
            target,
        };
        reads.acquire_external(target);
        let mut changes = self.changes.subscribe();
        let value = match reads.refresh_external(target).await {
            Ok(()) => reads.read(target).await,
            Err(error) => Err(error),
        };
        if let Err(error) = self.start(client, target) {
            return Err(convert(error));
        }
        if let Err(error) = self.reconcile_watches() {
            let _ = self.stop(client, target);
            return Err(error);
        }
        if !self.clients.lock().contains_key(client) {
            return Err(convert(SubscriptionError::StreamEnded));
        }
        if let Err(error) = match value {
            Ok(value) => self.publisher.publish_initial(target, value),
            Err(error) => self.publisher.publish_failure(target, error),
        } {
            let _ = self.stop(client, target);
            return Err(convert(error));
        }
        let mut workers = self.workers.lock();
        if workers.contains_key(target) {
            return Ok(());
        }
        let (waiting_sender, mut waiting_changes) = tokio::sync::mpsc::unbounded_channel();
        self.waiting_workers
            .lock()
            .insert(target.clone(), waiting_sender);
        let publisher = self.publisher.clone();
        let timer = self.timer.clone();
        let worker_target = target.clone();
        let usecase = self.clone();
        let task = tokio::spawn(async move {
            let mut interval =
                timer.interval(crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration());
            loop {
                // 定期、Repository の増減（Workspaces）、対象 Repository の Notion 設定の変化で外部の情報を取り直す。
                // 取りこぼしは、読み直すだけにする。
                let mut refresh_external = false;
                let mut completed = Vec::new();
                tokio::select! {
                    result = changes.recv() => match result {
                        Ok(change) if change.skip.as_ref() == Some(&worker_target) => continue,
                        Ok(change) if worker_target.affected_by(&change.source) => {
                            refresh_external = adds_external_information(&worker_target, &change.source);
                        }
                        Ok(_) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    result = waiting_changes.recv() => match result {
                        Some(change) => completed.push(change.completed),
                        None => break,
                    },
                    _ = interval.next(), if worker_target.external_information() => {
                        refresh_external = true;
                    }
                }
                // 読むのは 1 回で足りるので、溜まった知らせは読む前にまとめる。
                loop {
                    use tokio::sync::broadcast::error::TryRecvError;
                    match changes.try_recv() {
                        Ok(change) => {
                            refresh_external |=
                                adds_external_information(&worker_target, &change.source);
                        }
                        Err(TryRecvError::Lagged(_)) => continue,
                        Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                    }
                }
                while let Ok(change) = waiting_changes.try_recv() {
                    completed.push(change.completed);
                }
                let refresh = if refresh_external {
                    reads.refresh_external(&worker_target).await
                } else {
                    Ok(())
                };
                if worker_target == SubscriptionTarget::Workspaces {
                    if let Err(error) = usecase.reconcile_watches() {
                        log::error!("State watch update failed: {error}");
                    }
                }
                match match refresh {
                    Ok(()) => reads.read(&worker_target).await,
                    Err(error) => Err(error),
                } {
                    Ok(value) => {
                        if let Err(error) = publisher.publish(&worker_target, value, None) {
                            log::error!("State publication failed: {error}");
                        }
                    }
                    Err(error) => {
                        if let Err(error) = publisher.publish_failure(&worker_target, error) {
                            log::error!("State failure publication failed: {error}");
                        }
                    }
                }
                for completed in completed {
                    let _ = completed.send(());
                }
            }
        });
        workers.insert(target.clone(), task);
        Ok(())
    }

    fn release_inactive(&self, target: &SubscriptionTarget) {
        let starting = self.starting_target.lock();
        if starting.as_ref() == Some(target) {
            return;
        }
        let clients = self.clients.lock();
        if !clients.values().any(|targets| targets.contains(target)) {
            if let Some(reads) = &self.reads {
                reads.release_external(target);
            }
        }
    }

    fn reconcile_watches(&self) -> Result<(), StateReadError> {
        let mut failure = None;
        if let (Some(reads), Some(watcher)) = (&self.reads, &self.watchers) {
            let mut watches = self.watches.lock();
            let repositories = reads.repositories();
            let review_comments_dir = reads.review_comments_dir();
            let workflows_dir = reads.workflows_dir();
            let required: std::collections::HashSet<_> = self
                .active_targets()
                .into_iter()
                .flat_map(|target| {
                    target.watches(
                        &repositories,
                        &self.history_paths,
                        &review_comments_dir,
                        &workflows_dir,
                        &self.hook_health_markers,
                    )
                })
                .collect();
            let current: std::collections::HashSet<_> = watches.keys().cloned().collect();
            let start: Vec<_> = required.difference(&current).cloned().collect();
            let stop: Vec<_> = current.difference(&required).cloned().collect();
            for requirement in stop {
                if let Some(id) = watches.remove(&requirement) {
                    if let Err(error) = watcher.stop(id) {
                        log::error!("State watch cleanup failed: {error}");
                    }
                }
            }
            for requirement in start {
                let result = match &requirement {
                    crate::usecase::state_subscription::WatchRequirement::Git(path) => {
                        watcher.start_git_dir(path)
                    }
                    crate::usecase::state_subscription::WatchRequirement::Files(path, source) => {
                        let subscriptions = self.clone();
                        let source = source.clone();
                        watcher.start_files(
                            path,
                            Arc::new(move || subscriptions.notify(source.clone())),
                        )
                    }
                };
                match result {
                    Ok(id) => {
                        watches.insert(requirement, id);
                    }
                    Err(error) => {
                        failure = Some(StateReadError::from_error(error));
                    }
                }
            }
        }
        let mut workers = self.workers.lock();
        let active = self.active_targets();
        workers.retain(|target, task| {
            if active.contains(target) {
                true
            } else {
                self.waiting_workers.lock().remove(target);
                task.abort();
                self.release_inactive(target);
                false
            }
        });
        failure.map_or(Ok(()), Err)
    }

    #[cfg(test)]
    pub fn publisher(&self) -> StateSubscriptionOutputRef {
        self.publisher.clone()
    }

    pub(crate) fn notify(&self, source: StateChangeSource) {
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
        #[cfg(test)]
        let _ = self.test_changes.send(source.clone());
        let _ = self.changes.send(StateChange { source, skip });
    }

    #[cfg(test)]
    pub(crate) fn changes(&self) -> tokio::sync::broadcast::Receiver<StateChangeSource> {
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
        subscriptions.insert(target.clone());
        Ok(())
    }

    pub async fn stop_read(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        let _start = self.starts.lock().await;
        self.stop(client, target)
    }

    pub fn stop(&self, client: &str, target: &SubscriptionTarget) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        let subscriptions = clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?;
        subscriptions.remove(target);
        drop(clients);
        if let Err(error) = self.reconcile_watches() {
            log::error!("State watch cleanup failed: {error}");
        }
        Ok(())
    }

    pub(crate) fn open_client(&self, id: String) -> Result<(), SubscriptionError> {
        let mut clients = self.clients.lock();
        if clients.contains_key(&id) {
            return Err(SubscriptionError::AlreadyExists);
        }
        clients.insert(id, Default::default());
        Ok(())
    }

    pub(crate) fn close_client(&self, id: &str) {
        self.clients.lock().remove(id);
        if let Err(error) = self.reconcile_watches() {
            log::error!("State stream cleanup failed: {error}");
        }
    }

    #[cfg(test)]
    pub(crate) fn output_ref(&self) -> &dyn StateSubscriptionOutput {
        self.publisher.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn test_worker_count(&self) -> usize {
        self.workers.lock().len()
    }

    #[cfg(test)]
    pub(crate) fn test_watches(&self) -> std::collections::HashMap<WatchRequirement, u64> {
        self.watches.lock().clone()
    }

    pub(crate) fn active_targets(&self) -> std::collections::HashSet<SubscriptionTarget> {
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
            .flat_map(|targets| targets.iter().cloned())
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
        if !clients.values().any(|targets| targets.contains(target)) {
            return Ok(false);
        }
        clients
            .get_mut(client)
            .ok_or(SubscriptionError::StreamEnded)?
            .insert(target.clone());
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
