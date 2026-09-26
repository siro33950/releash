mod reads;
#[cfg(test)]
pub(crate) use reads::reads_tests::Fixture as StateReadsFixture;
pub(crate) use reads::StateSubscriptionRead;
pub(crate) use reads::{StateReadError, StateReadFailure, WorkspaceStateReads};
mod terminal;
mod value;
use crate::domain::state_subscription::{
    Delivery, Event, SubscriptionError, Subscriptions, Version,
};
use futures_util::{Stream, StreamExt};
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::Notify;
pub(crate) use value::StateValue;

pub(crate) const REPO_PATHS: &str = "repository-paths";
pub(crate) const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) trait SubscriptionTimer: Send + Sync {
    fn interval(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>>;
}

pub(crate) enum StateSubscriptionEvent {
    Ready,
    Item(String, Event<StateValue>),
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionUsecase {
    publisher: StateSubscriptionPublisher,
    terminal:
        Option<Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>>,
    terminal_inputs: Arc<Mutex<std::collections::HashMap<(String, String), String>>>,
    timer: Arc<dyn SubscriptionTimer>,
    history_paths: Vec<String>,
    reads: Option<Arc<dyn StateSubscriptionRead>>,
    watchers: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
    workers: Arc<
        Mutex<
            std::collections::HashMap<
                crate::domain::state_subscription::SubscriptionTarget,
                tokio::task::JoinHandle<()>,
            >,
        >,
    >,
    starts: Arc<tokio::sync::Mutex<()>>,
    watches: Arc<
        Mutex<std::collections::HashMap<crate::domain::state_subscription::WatchRequirement, u64>>,
    >,
}

impl StateSubscriptionUsecase {
    pub fn new(paths: Vec<String>, timer: Arc<dyn SubscriptionTimer>) -> Self {
        let mut state = Subscriptions::new(uuid::Uuid::new_v4().to_string());
        state
            .register(
                REPO_PATHS.into(),
                StateValue::RepositoryPaths(paths),
                Delivery::Full,
            )
            .expect("unique target");
        let terminal_inputs = Arc::new(Mutex::new(std::collections::HashMap::new()));
        Self {
            publisher: StateSubscriptionPublisher {
                state: Arc::new(Mutex::new(state)),
                changed: Arc::new(Notify::new()),
                invalidated: tokio::sync::broadcast::channel(64).0,
                terminal_routes: Default::default(),
                terminal_inputs: terminal_inputs.clone(),
                boot: uuid::Uuid::new_v4().to_string(),
            },
            timer,
            terminal: None,
            terminal_inputs,
            reads: None,
            history_paths: vec![],
            watchers: None,
            workers: Default::default(),
            watches: Default::default(),
            starts: Default::default(),
        }
    }

    pub fn with_terminal(
        mut self,
        terminal: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    ) -> Self {
        terminal.connect_state(Arc::new(self.publisher.clone()));
        self.terminal = Some(terminal);
        self
    }

    pub fn with_reads(
        mut self,
        reads: Arc<dyn StateSubscriptionRead>,
        watcher: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
        history_paths: Vec<String>,
    ) -> Self {
        self.history_paths = history_paths;
        self.reads = Some(reads);
        self.watchers = watcher;
        self
    }

    pub async fn start_read(
        &self,
        client: &str,
        raw: &str,
        version: Option<&Version>,
    ) -> Result<(), StateReadError> {
        use crate::domain::state_subscription::{StateChangeSource, SubscriptionTarget};
        let convert = StateReadError::from_error;
        let target = SubscriptionTarget::parse(raw).map_err(convert)?;
        if let SubscriptionTarget::Terminal(_) = &target {
            return self.start_terminal(client, raw, version, client).await;
        }
        if target == SubscriptionTarget::RepositoryPaths {
            return self.start(client, raw, version).map_err(convert);
        }
        let reads = self
            .reads
            .clone()
            .ok_or_else(|| convert(SubscriptionError::UnknownTarget))?;
        // ponytail: subscription starts are serialized; split by target if initial reads contend.
        let _start = self.starts.lock().await;
        {
            let mut state = self.publisher.state.lock();
            if state.active_targets().contains(&target) {
                state.start(client, raw, version).map_err(convert)?;
                self.publisher.changed.notify_waiters();
                return Ok(());
            }
        }
        let mut changes = self.publisher.invalidated.subscribe();
        if target == SubscriptionTarget::Workspaces
            && !self
                .publisher
                .state
                .lock()
                .active_targets()
                .contains(&target)
        {
            reads
                .refresh_workspaces(Some(StateChangeSource::Repositories))
                .await;
        }
        reads.refresh_external(&target).await?;
        let value = reads.read(&target).await?;
        {
            let mut state = self.publisher.state.lock();
            state
                .start_with_snapshot(client, raw, value, version)
                .map_err(convert)?;
        }
        self.publisher.changed.notify_waiters();
        if let Err(error) = self.reconcile_watches() {
            let _ = self.stop(client, raw);
            return Err(error);
        }
        let mut state = self.publisher.state.lock();
        state.ensure_active(&target).map_err(convert)?;
        let mut workers = self.workers.lock();
        if workers.contains_key(&target) {
            return Ok(());
        }
        let publisher = self.publisher.clone();
        let timer = self.timer.clone();
        let worker_target = target.clone();
        let usecase = self.clone();
        let task = tokio::spawn(async move {
            let mut interval =
                timer.interval(crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration());
            loop {
                let source = tokio::select! {
                    result = changes.recv() => match result {
                        Ok(source) if worker_target.affected_by(&source)
                            || matches!((&worker_target, &source),
                                (SubscriptionTarget::Failures(_, _),
                                 crate::domain::state_subscription::StateChangeSource::Failures(_))) => Some(source),
                        Ok(_) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = interval.next(), if worker_target.external_information() => None,
                };
                if source.is_none() {
                    if let Err(error) = reads.refresh_external(&worker_target).await {
                        log::warn!("External state refresh failed: {error}");
                        continue;
                    }
                }
                if worker_target == SubscriptionTarget::Workspaces {
                    reads.refresh_workspaces(source).await;
                    if let Err(error) = usecase.reconcile_watches() {
                        log::error!("State watch update failed: {error}");
                    }
                }
                match reads.read(&worker_target).await {
                    Ok(value) => {
                        if let Err(error) =
                            publisher.publish(&worker_target.to_string(), value, None)
                        {
                            log::error!("State publication failed: {error}");
                        }
                    }
                    Err(error) => log::warn!("State read failed for {worker_target}: {error}"),
                }
            }
        });
        workers.insert(target, task);
        Ok(())
    }

    fn reconcile_watches(&self) -> Result<(), StateReadError> {
        let mut failure = None;
        if let (Some(reads), Some(watcher)) = (&self.reads, &self.watchers) {
            let mut watches = self.watches.lock();
            let (start, stop) = self
                .publisher
                .state
                .lock()
                .watch_changes(&reads.repositories(), &self.history_paths);
            for requirement in stop {
                if let Some(id) = watches.remove(&requirement) {
                    if let Err(error) = watcher.stop(id) {
                        log::error!("State watch cleanup failed: {error}");
                    }
                }
            }
            for requirement in start {
                let result = match &requirement {
                    crate::domain::state_subscription::WatchRequirement::Git(path) => {
                        watcher.start_git_dir(path)
                    }
                    crate::domain::state_subscription::WatchRequirement::Files(path) => {
                        watcher.start_files(path)
                    }
                };
                match result {
                    Ok(id) => {
                        watches.insert(requirement, id);
                    }
                    Err(error) => {
                        self.publisher.state.lock().watch_failed(&requirement);
                        failure = Some(StateReadError::from_error(error));
                    }
                }
            }
        }
        let mut state = self.publisher.state.lock();
        let active = state.active_targets();
        self.workers.lock().retain(|target, task| {
            if active.contains(target) {
                true
            } else {
                task.abort();
                false
            }
        });
        state.release_inactive_snapshots();
        failure.map_or(Ok(()), Err)
    }

    pub fn publisher(&self) -> StateSubscriptionPublisher {
        self.publisher.clone()
    }

    pub fn start(
        &self,
        client: &str,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), SubscriptionError> {
        self.publisher
            .update(|state| state.start(client, target, version))
    }

    pub async fn stop_read(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        let _start = self.starts.lock().await;
        self.stop(client, target)
    }

    pub fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        self.publisher.update(|state| state.stop(client, target))?;
        self.stop_terminal(client, target);
        if let Err(error) = self.reconcile_watches() {
            log::error!("State watch cleanup failed: {error}");
        }
        Ok(())
    }

    pub fn open(
        &self,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        self.publisher.state.lock().open(id.clone())?;
        let permit = StreamPermit {
            usecase: self.clone(),
            id,
        };
        let timer = self.timer.interval(BOOKMARK_INTERVAL);
        let events = futures_util::stream::unfold(
            (permit, timer),
            |(permit, mut timer)| async move {
                loop {
                    let notify = permit.usecase.publisher.changed.clone();
                    let changed = notify.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let requests = permit
                        .usecase
                        .publisher
                        .state
                        .lock()
                        .snapshot_requests(&permit.id);
                    for raw in requests {
                        let Ok(target) =
                            crate::domain::state_subscription::SubscriptionTarget::parse(&raw)
                        else {
                            continue;
                        };
                        let mut workers = permit.usecase.workers.lock();
                        if workers.get(&target).is_some_and(|task| !task.is_finished()) {
                            continue;
                        }
                        let usecase = permit.usecase.clone();
                        workers.insert(
                            target,
                            tokio::spawn(async move {
                                match usecase.refresh_terminal(&raw).await {
                                    Ok(()) => usecase.publisher.changed.notify_waiters(),
                                    Err(error) => log::error!("Terminal snapshot failed: {error}"),
                                }
                            }),
                        );
                    }
                    let item = permit.usecase.publisher.state.lock().next(&permit.id);
                    if let Some((target, event)) = item {
                        return Some((
                            StateSubscriptionEvent::Item(target, event),
                            (permit, timer),
                        ));
                    }
                    tokio::select! {
                        _ = changed => {},
                        _ = timer.next() => permit.usecase.publisher.state.lock().bookmark(&permit.id),
                    }
                }
            },
        );
        Ok(futures_util::stream::once(async { StateSubscriptionEvent::Ready }).chain(events))
    }
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionPublisher {
    state: Arc<Mutex<Subscriptions<StateValue>>>,
    changed: Arc<Notify>,
    terminal_routes: Arc<Mutex<std::collections::HashMap<String, String>>>,
    terminal_inputs: Arc<Mutex<std::collections::HashMap<(String, String), String>>>,
    boot: String,
    invalidated:
        tokio::sync::broadcast::Sender<crate::domain::state_subscription::StateChangeSource>,
}

impl StateSubscriptionPublisher {
    #[cfg(test)]
    pub(crate) fn subscribe_changes(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::domain::state_subscription::StateChangeSource>
    {
        self.invalidated.subscribe()
    }

    #[cfg(any(test, all(debug_assertions, feature = "desktop")))]
    pub(crate) fn for_test() -> Self {
        StateSubscriptionUsecase::new(
            vec![],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        )
        .publisher()
    }

    pub fn invalidate(&self, source: crate::domain::state_subscription::StateChangeSource) {
        let _ = self.invalidated.send(source);
    }

    fn update(
        &self,
        update: impl FnOnce(&mut Subscriptions<StateValue>) -> Result<(), SubscriptionError>,
    ) -> Result<(), SubscriptionError> {
        update(&mut self.state.lock())?;
        self.changed.notify_waiters();
        Ok(())
    }

    pub fn publish(
        &self,
        target: &str,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.update(|state| state.publish(target, snapshot, delta))
    }
}

struct StreamPermit {
    usecase: StateSubscriptionUsecase,
    id: String,
}
impl Drop for StreamPermit {
    fn drop(&mut self) {
        self.usecase.publisher.state.lock().close(&self.id);
        let targets: Vec<_> = self
            .usecase
            .terminal_inputs
            .lock()
            .keys()
            .filter(|(client, _)| client == &self.id)
            .map(|(_, target)| target.clone())
            .collect();
        for target in targets {
            self.usecase.stop_terminal(&self.id, &target);
        }
        if let Err(error) = self.usecase.reconcile_watches() {
            log::error!("State stream cleanup failed: {error}");
        }
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
