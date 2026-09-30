#[path = "../../tests/state_subscription_reads/reads_test.rs"]
mod reads_tests;

pub(crate) use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
pub(crate) use crate::infrastructure::state_subscription::{Delivery, Event, Version};
pub(crate) use reads_tests::Fixture as StateReadsFixture;

pub(crate) struct WakeFlag(pub(crate) std::sync::atomic::AtomicBool);

impl std::task::Wake for WakeFlag {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(crate) fn registration(
    session_key: &str,
    workspace_path: &str,
    session_id: Option<&str>,
    runtime_generation: u64,
    latest_sequence: u64,
) -> crate::usecase::terminal_surface::output::TerminalRegistration {
    crate::usecase::terminal_surface::output::TerminalRegistration {
        session_key: session_key.into(),
        workspace_path: workspace_path.into(),
        session_id: session_id.map(str::to_owned),
        runtime_generation,
        latest_sequence,
    }
}

pub(crate) fn start(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.start(client, &typed)?;
    usecase
        .test_presenter()
        .expect("test presenter")
        .start(client, target, cursor)
}

pub(crate) async fn start_read(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(crate::usecase::state_subscription::StateReadError::from_error)?;
    usecase
        .start_subscription(client, &typed, None, cursor)
        .await
}

pub(crate) fn stop(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.stop(client, &typed)?;
    usecase.with_active_targets(|active| usecase.publisher().stop(client, &typed, active))
}

pub(crate) async fn stop_read(
    usecase: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    client: &str,
    target: &str,
) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)?;
    usecase.stop_subscription(client, &typed).await
}

pub(crate) async fn start_terminal(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
    input_id: &str,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    use crate::usecase::state_subscription::StateReadError;
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(StateReadError::from_error)?;
    usecase
        .start_subscription(client, &typed, Some(input_id), cursor)
        .await
}

pub(crate) fn terminal_processed(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    target: &str,
    units: usize,
) -> Result<(), crate::usecase::state_subscription::StateReadError> {
    use crate::usecase::state_subscription::{StateReadError, StateReadFailure};
    if units
        != usecase
            .test_presenter()
            .expect("test presenter")
            .terminal_report_units()
    {
        return Err(StateReadError {
            source: StateReadFailure::InvalidTerminalInput,
            message: "Invalid terminal processed units".into(),
        });
    }
    let typed = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(StateReadError::from_error)?;
    usecase.terminal_processed(client, &typed, units)
}

pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    crate::adaptor::presenter::state_subscription::test_output()
}

pub(crate) fn test_subscriptions() -> crate::usecase::state_subscription::StateSubscriptionUsecase {
    crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        test_output(),
        std::sync::Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
}

pub(crate) fn changes(
    subscriptions: &crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource> {
    subscriptions.changes()
}

pub(crate) fn take_changes(
    receiver: &mut tokio::sync::broadcast::Receiver<
        crate::usecase::state_subscription::StateChangeSource,
    >,
) -> Vec<crate::usecase::state_subscription::StateChangeSource> {
    let mut changes = Vec::new();
    while let Ok(change) = receiver.try_recv() {
        changes.push(change);
    }
    changes
}

pub(crate) struct CapturingNotifier<T> {
    changes: std::sync::Mutex<
        tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource>,
    >,
    values: std::sync::Mutex<Vec<T>>,
    extract: fn(crate::usecase::state_subscription::StateChangeSource) -> Option<T>,
}

impl<T> CapturingNotifier<T> {
    fn new(
        subscriptions: &crate::usecase::state_subscription::StateSubscriptionUsecase,
        extract: fn(crate::usecase::state_subscription::StateChangeSource) -> Option<T>,
    ) -> Self {
        Self {
            changes: std::sync::Mutex::new(subscriptions.changes()),
            values: std::sync::Mutex::new(Vec::new()),
            extract,
        }
    }

    pub(crate) fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Vec<T>>> {
        let mut values = self.values.lock()?;
        let mut changes = self.changes.lock().expect("captured state changes");
        values.extend(
            take_changes(&mut changes)
                .into_iter()
                .filter_map(self.extract),
        );
        Ok(values)
    }

    pub(crate) fn take(&self) -> Vec<T> {
        std::mem::take(&mut self.lock().expect("captured state changes"))
    }
}

impl CapturingNotifier<Vec<String>> {
    pub(crate) fn repositories(
        subscriptions: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        Self::new(subscriptions, |change| match change {
            crate::usecase::state_subscription::StateChangeSource::Repository(paths) => Some(paths),
            _ => None,
        })
    }
}

impl CapturingNotifier<String> {
    pub(crate) fn worktrees(
        subscriptions: &crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        Self::new(subscriptions, |change| match change {
            crate::usecase::state_subscription::StateChangeSource::Worktree(path) => Some(path),
            _ => None,
        })
    }
}

pub(crate) fn same(
    value: &crate::adaptor::presenter::client::StatePayload,
    expected: impl std::borrow::Borrow<crate::usecase::state_subscription::StateValue>,
) -> bool {
    *value
        == crate::adaptor::presenter::state_subscription_wire::payload(expected.borrow()).unwrap()
}

pub(crate) fn payload(
    value: &crate::usecase::state_subscription::StateValue,
) -> Result<crate::adaptor::presenter::client::StatePayload, connectrpc::ConnectError> {
    crate::adaptor::presenter::state_subscription_wire::payload(value)
}

pub(crate) fn terminal_item(
    value: &crate::adaptor::presenter::client::StatePayload,
) -> &crate::adaptor::presenter::client::terminal_event::Item {
    use crate::adaptor::presenter::client::state_payload::Value;
    let Some(Value::Terminal(event)) = &value.value else {
        panic!("terminal payload");
    };
    event.item.as_ref().expect("terminal item")
}

use crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter;
use crate::usecase::state_subscription::{StateSubscriptionUsecase, SubscriptionError};
use futures_util::Stream;
use std::sync::Arc;

#[cfg(test)]
impl StateSubscriptionUsecase {
    pub(crate) fn new(
        paths: Vec<String>,
        timer: Arc<dyn crate::usecase::state_subscription::SubscriptionTimer>,
    ) -> Self {
        let presenter = Arc::new(StateSubscriptionPresenter::new());
        let paths = Arc::new(parking_lot::RwLock::new(paths));
        let mut usecase = Self::new_with_output(presenter, timer);
        usecase.reads = Some(Arc::new(TestRepositoryPathsReads(paths.clone())));
        usecase.test_repository_paths = Some(paths);
        usecase
    }

    pub(crate) fn test_set_repository_paths(&self, paths: Vec<String>) {
        *self
            .test_repository_paths
            .as_ref()
            .expect("test paths")
            .write() = paths;
    }

    pub(crate) fn test_presenter(&self) -> Option<&StateSubscriptionPresenter> {
        self.output_ref().as_any().downcast_ref()
    }

    pub(crate) fn open(
        &self,
        id: String,
    ) -> Result<
        impl Stream<Item = crate::adaptor::presenter::state_subscription::StateSubscriptionEvent>
            + Send
            + use<>,
        SubscriptionError,
    > {
        self.test_presenter()
            .expect("test presenter")
            .stream(self.clone(), id)
    }
}

struct TestRepositoryPathsReads(Arc<parking_lot::RwLock<Vec<String>>>);

#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for TestRepositoryPathsReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        match target {
            crate::usecase::state_subscription::SubscriptionTarget::RepositoryPaths => Ok(
                crate::usecase::state_subscription::StateValue::RepositoryPaths(
                    self.0.read().clone(),
                ),
            ),
            _ => Err(
                crate::usecase::state_subscription::StateReadError::from_error(
                    crate::usecase::state_subscription::SubscriptionError::UnknownTarget,
                ),
            ),
        }
    }

    fn repositories(&self) -> Vec<String> {
        self.0.read().clone()
    }
}
