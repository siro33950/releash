mod error;
mod reads;
pub(crate) use reads::StateSubscriptionRead;
pub(crate) use reads::{StateReadError, StateReadFailure, WorkspaceStateReads};
mod target;
mod terminal;
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
    fn invalidate(&self, source: StateChangeSource);
    fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
        terminal_input_id: Option<&str>,
    ) -> Result<(), StateReadError>;
    fn stop(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        active: &std::collections::HashSet<SubscriptionTarget>,
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
    fn set_terminal_snapshot(
        &self,
        target: &SubscriptionTarget,
        runtime_generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError>;
}

pub(crate) type StateSubscriptionOutputRef = Arc<dyn StateSubscriptionOutput>;

#[derive(Clone)]
pub(crate) struct StateSubscriptionUsecase {
    publisher: StateSubscriptionOutputRef,
    changes: tokio::sync::broadcast::Sender<StateChangeSource>,
    clients: Arc<
        Mutex<std::collections::HashMap<String, std::collections::HashSet<SubscriptionTarget>>>,
    >,
    terminal_inputs: Arc<Mutex<std::collections::HashMap<(String, SubscriptionTarget), String>>>,
    terminal_resets: Arc<
        Mutex<std::collections::HashMap<SubscriptionTarget, std::collections::HashSet<String>>>,
    >,
    terminal:
        Option<Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>>,
    timer: Arc<dyn SubscriptionTimer>,
    history_paths: Vec<String>,
    reads: Option<Arc<dyn StateSubscriptionRead>>,
    watchers: Option<Arc<crate::usecase::watcher::WatcherUsecase>>,
    workers: Arc<Mutex<std::collections::HashMap<SubscriptionTarget, tokio::task::JoinHandle<()>>>>,
    starts: Arc<tokio::sync::Mutex<()>>,
    watches: Arc<Mutex<std::collections::HashMap<WatchRequirement, u64>>>,
}

impl StateSubscriptionUsecase {
    pub(crate) async fn start_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        terminal_input_id: Option<&str>,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        if let Some(input_id) = terminal_input_id
            .or_else(|| matches!(target, SubscriptionTarget::Terminal(_)).then_some(client))
        {
            self.start_terminal(client, target, input_id, cursor)
                .await?;
        } else {
            self.start_read(client, target).await?;
            if let Err(error) = self.publisher.start(client, target, cursor, None) {
                let _ = self.stop(client, target);
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) async fn stop_subscription(
        &self,
        client: &str,
        target: &SubscriptionTarget,
    ) -> Result<(), SubscriptionError> {
        self.stop_read(client, target).await?;
        self.publisher.stop(client, target, &self.active_targets())
    }

    pub fn new_with_output(
        publisher: StateSubscriptionOutputRef,
        changes: tokio::sync::broadcast::Sender<StateChangeSource>,
        timer: Arc<dyn SubscriptionTimer>,
    ) -> Self {
        Self {
            publisher,
            changes,
            clients: Default::default(),
            terminal_inputs: Default::default(),
            terminal_resets: Default::default(),
            timer,
            terminal: None,
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
        target: &SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        let convert = StateReadError::from_error;
        if let SubscriptionTarget::Terminal(_) = target {
            return self.start_terminal(client, target, client, None).await;
        }
        if *target == SubscriptionTarget::RepositoryPaths {
            return self.start(client, target).map_err(convert);
        }
        let reads = self
            .reads
            .clone()
            .ok_or_else(|| convert(SubscriptionError::UnknownTarget))?;
        // ponytail: subscription starts are serialized; split by target if initial reads contend.
        let _start = self.starts.lock().await;
        if self.active_targets().contains(target) {
            self.start(client, target).map_err(convert)?;
            return Ok(());
        }
        let mut changes = self.changes.subscribe();
        if *target == SubscriptionTarget::Workspaces && !self.active_targets().contains(target) {
            reads
                .refresh_workspaces(Some(StateChangeSource::Repositories))
                .await;
        }
        reads.refresh_external(target).await?;
        let value = reads.read(target).await?;
        self.start(client, target).map_err(convert)?;
        if let Err(error) = self.reconcile_watches() {
            let _ = self.stop(client, target);
            return Err(error);
        }
        if !self.clients.lock().contains_key(client) {
            return Err(convert(SubscriptionError::StreamEnded));
        }
        if let Err(error) = self.publisher.publish_initial(target, value) {
            let _ = self.stop(client, target);
            return Err(convert(error));
        }
        let mut workers = self.workers.lock();
        if workers.contains_key(target) {
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
                                 crate::usecase::state_subscription::StateChangeSource::Failures(_))) => Some(source),
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
                        if let Err(error) = publisher.publish(&worker_target, value, None) {
                            log::error!("State publication failed: {error}");
                        }
                    }
                    Err(error) => log::warn!("State read failed for {worker_target}: {error}"),
                }
            }
        });
        workers.insert(target.clone(), task);
        Ok(())
    }

    fn reconcile_watches(&self) -> Result<(), StateReadError> {
        let mut failure = None;
        if let (Some(reads), Some(watcher)) = (&self.reads, &self.watchers) {
            let mut watches = self.watches.lock();
            let repositories = reads.repositories();
            let required: std::collections::HashSet<_> = self
                .active_targets()
                .into_iter()
                .flat_map(|target| target.watches(&repositories, &self.history_paths))
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
                    crate::usecase::state_subscription::WatchRequirement::Files(path) => {
                        watcher.start_files(path)
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
        let active = self.active_targets();
        self.workers.lock().retain(|target, task| {
            if active.contains(target) {
                true
            } else {
                task.abort();
                false
            }
        });
        failure.map_or(Ok(()), Err)
    }

    pub fn publisher(&self) -> StateSubscriptionOutputRef {
        self.publisher.clone()
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
        self.stop_terminal(client, target);
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
        let targets = self
            .clients
            .lock()
            .remove(id)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        for target in targets {
            self.stop_terminal(id, &target);
        }
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

    pub(crate) fn schedule_terminal_refresh(
        &self,
        clients: Vec<String>,
        target: SubscriptionTarget,
    ) {
        self.terminal_resets
            .lock()
            .entry(target.clone())
            .or_default()
            .extend(clients);
        let mut workers = self.workers.lock();
        if workers.get(&target).is_some_and(|task| !task.is_finished()) {
            return;
        }
        let usecase = self.clone();
        workers.insert(
            target.clone(),
            tokio::spawn(async move {
                loop {
                    let result = usecase.refresh_terminal(&target).await;
                    if let Err(error) = &result {
                        log::error!("Terminal snapshot failed: {error}");
                    }
                    let resets = usecase.terminal_resets.lock();
                    if result.is_err()
                        || resets
                            .get(&target)
                            .is_none_or(std::collections::HashSet::is_empty)
                    {
                        usecase.workers.lock().remove(&target);
                        break;
                    }
                }
            }),
        );
    }

    pub(crate) fn active_targets(&self) -> std::collections::HashSet<SubscriptionTarget> {
        self.clients
            .lock()
            .values()
            .flat_map(|targets| targets.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
