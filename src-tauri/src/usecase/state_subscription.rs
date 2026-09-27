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

pub(crate) const REPO_PATHS: &str = "repository-paths";

pub(crate) trait SubscriptionTimer: Send + Sync {
    fn interval(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>>;
}

pub(crate) trait StateSubscriptionOutput: Send + Sync {
    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any;
    fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<StateChangeSource>;
    fn invalidate(&self, source: StateChangeSource);
    fn publish(
        &self,
        target: &str,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError>;
    fn open(&self, client: String) -> Result<(), SubscriptionError>;
    fn close(&self, client: &str);
    fn start(
        &self,
        client: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError>;
    fn start_with_snapshot(
        &self,
        client: &str,
        target: &str,
        snapshot: StateValue,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError>;
    fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError>;
    fn active_targets(&self) -> std::collections::HashSet<String>;
    fn ensure_active(&self, target: &str) -> Result<(), SubscriptionError>;
    fn release_inactive_snapshots(&self);
    fn needs_snapshot(
        &self,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<bool, SubscriptionError>;
    fn terminal_pending_amount(&self, client: &str, target: &str) -> usize;
    fn set_terminal_snapshot(
        &self,
        target: &str,
        runtime_generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError>;
    fn terminal_reset_clients(&self, target: &str) -> Vec<String>;
    fn terminal_report_units(&self) -> usize;
    fn set_terminal_input(&self, client: &str, target: &str, input_id: &str);
    fn remove_terminal_input(&self, client: &str, target: &str) -> Option<String>;
    fn has_terminal_input(&self, client: &str, target: &str) -> bool;
    fn terminal_targets(&self, client: &str) -> Vec<String>;
    fn terminal_state_sink(
        &self,
    ) -> Arc<dyn crate::usecase::terminal_surface::output::TerminalSurfaceStateSink>;
}

pub(crate) type StateSubscriptionOutputRef = Arc<dyn StateSubscriptionOutput>;

#[derive(Clone)]
pub(crate) struct StateSubscriptionUsecase {
    publisher: StateSubscriptionOutputRef,
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
    pub fn new_with_output(
        publisher: StateSubscriptionOutputRef,
        timer: Arc<dyn SubscriptionTimer>,
    ) -> Self {
        Self {
            publisher,
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
        terminal.connect_state(self.publisher.terminal_state_sink());
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
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        let convert = StateReadError::from_error;
        let target = SubscriptionTarget::parse(raw).map_err(convert)?;
        if let SubscriptionTarget::Terminal(_) = &target {
            return self.start_terminal(client, raw, cursor, client).await;
        }
        if target == SubscriptionTarget::RepositoryPaths {
            return self.start(client, raw, cursor).map_err(convert);
        }
        let reads = self
            .reads
            .clone()
            .ok_or_else(|| convert(SubscriptionError::UnknownTarget))?;
        // ponytail: subscription starts are serialized; split by target if initial reads contend.
        let _start = self.starts.lock().await;
        if self.publisher.active_targets().contains(raw) {
            self.publisher.start(client, raw, cursor).map_err(convert)?;
            return Ok(());
        }
        let mut changes = self.publisher.subscribe_changes();
        if target == SubscriptionTarget::Workspaces
            && !self.publisher.active_targets().contains(raw)
        {
            reads
                .refresh_workspaces(Some(StateChangeSource::Repositories))
                .await;
        }
        reads.refresh_external(&target).await?;
        let value = reads.read(&target).await?;
        self.publisher
            .start_with_snapshot(client, raw, value, cursor)
            .map_err(convert)?;
        if let Err(error) = self.reconcile_watches() {
            let _ = self.stop(client, raw);
            return Err(error);
        }
        self.publisher.ensure_active(raw).map_err(convert)?;
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
            let repositories = reads.repositories();
            let required: std::collections::HashSet<_> = self
                .publisher
                .active_targets()
                .into_iter()
                .filter_map(|raw| SubscriptionTarget::parse(&raw).ok())
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
        let active = self.publisher.active_targets();
        self.workers.lock().retain(|target, task| {
            if active.contains(&target.to_string()) {
                true
            } else {
                task.abort();
                false
            }
        });
        self.publisher.release_inactive_snapshots();
        failure.map_or(Ok(()), Err)
    }

    pub fn publisher(&self) -> StateSubscriptionOutputRef {
        self.publisher.clone()
    }

    pub fn start(
        &self,
        client: &str,
        target: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), SubscriptionError> {
        SubscriptionTarget::parse(target)?;
        self.publisher.start(client, target, cursor)
    }

    pub async fn stop_read(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        let _start = self.starts.lock().await;
        self.stop(client, target)
    }

    pub fn stop(&self, client: &str, target: &str) -> Result<(), SubscriptionError> {
        SubscriptionTarget::parse(target)?;
        self.publisher.stop(client, target)?;
        self.stop_terminal(client, target);
        if let Err(error) = self.reconcile_watches() {
            log::error!("State watch cleanup failed: {error}");
        }
        Ok(())
    }

    pub(crate) fn open_client(&self, id: String) -> Result<(), SubscriptionError> {
        self.publisher.open(id)
    }

    pub(crate) fn close_client(&self, id: &str) {
        self.publisher.close(id);
        let targets = self.publisher.terminal_targets(id);
        for target in targets {
            self.stop_terminal(id, &target);
        }
        if let Err(error) = self.reconcile_watches() {
            log::error!("State stream cleanup failed: {error}");
        }
    }

    pub(crate) fn timer(&self) -> Arc<dyn SubscriptionTimer> {
        self.timer.clone()
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

    pub(crate) fn schedule_terminal_refresh(&self, raw: String) {
        let Ok(target) = SubscriptionTarget::parse(&raw) else {
            return;
        };
        let mut workers = self.workers.lock();
        if workers.get(&target).is_some_and(|task| !task.is_finished()) {
            return;
        }
        let usecase = self.clone();
        workers.insert(
            target,
            tokio::spawn(async move {
                if let Err(error) = usecase.refresh_terminal(&raw).await {
                    log::error!("Terminal snapshot failed: {error}");
                }
            }),
        );
    }
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
