#[path = "../../tests/state_subscription_reads/reads_test.rs"]
mod reads_tests;

pub(crate) use crate::adaptor::presenter::state_subscription::StateSubscriptionEvent;
pub(crate) use crate::infrastructure::state_subscription::{Delivery, Event};
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

pub(crate) fn terminal_processed(
    usecase: &TerminalSubscriptions,
    client: &str,
    target: &str,
    units: usize,
) -> Result<(), connectrpc::ConnectError> {
    use crate::adaptor::presenter::connect::classified_error;
    if units != crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::report_units() {
        return Err(classified_error(crate::adaptor::presenter::error::AppError::invalid_request("Invalid terminal processed units")));
    }
    let target = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(classified_error)?;
    let id = usecase
        .terminal
        .test_input_id(client, &target)
        .unwrap_or_default();
    usecase
        .terminal
        .terminal_processed(&id, units)
        .map_err(classified_error)
}

pub(crate) fn test_output() -> crate::usecase::state_subscription::StateSubscriptionOutputRef {
    crate::adaptor::presenter::state_subscription::test_output()
}

pub(crate) fn test_subscriptions() -> crate::usecase::state_subscription::StateSubscriptionUsecase {
    crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        test_output(),
        read_driver(),
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
    value: &crate::adaptor::presenter::state_subscription::PublishedState,
    expected: impl std::borrow::Borrow<crate::usecase::state_subscription::StateValue>,
) -> bool {
    *value
        == crate::adaptor::presenter::state_subscription::PublishedState::from(
            crate::adaptor::presenter::state_subscription_wire::payload(expected.borrow()).unwrap(),
        )
}

pub(crate) fn payload(
    value: &crate::usecase::state_subscription::StateValue,
) -> Result<crate::adaptor::presenter::state_subscription::PublishedState, connectrpc::ConnectError>
{
    crate::adaptor::presenter::state_subscription_wire::payload(value)
        .map(crate::adaptor::presenter::state_subscription::PublishedState::from)
}

pub(crate) fn terminal_item(
    value: &crate::adaptor::presenter::state_subscription::PublishedState,
) -> &crate::adaptor::presenter::client::terminal_event::Item {
    let crate::adaptor::presenter::state_subscription::PublishedState::Value(value) = value else {
        panic!("terminal state");
    };
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
        timer: tokio::sync::mpsc::UnboundedSender<crate::usecase::state_subscription::ReadWorker>,
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
        deps(
            self.clone(),
            Arc::new(self.test_presenter().expect("test presenter").clone()),
        )
        .stream(id)
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

pub(crate) fn deps(
    usecase: StateSubscriptionUsecase,
    presenter: Arc<StateSubscriptionPresenter>,
) -> crate::adaptor::controller::api::StateSubscriptionDeps {
    let output = Arc::new(
        crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
            &presenter,
        ),
    );
    let terminal = crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase::new(
        output,
        None,
        crate::test_support::state_subscription::terminal_driver(),
    );

    crate::adaptor::controller::api::StateSubscriptionDeps::new(usecase, presenter, terminal)
}

#[derive(Clone)]
pub(crate) struct TerminalSubscriptions {
    pub(crate) usecase: StateSubscriptionUsecase,
    pub(crate) terminal:
        crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase,
    pub(crate) presenter:
        Arc<crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter>,
}

impl StateSubscriptionUsecase {
    pub(crate) fn deps(&self) -> crate::adaptor::controller::api::StateSubscriptionDeps {
        deps(
            self.clone(),
            Arc::new(self.test_presenter().unwrap().clone()),
        )
    }

    pub(crate) fn with_terminal(
        self,
        terminal: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    ) -> TerminalSubscriptions {
        let presenter = Arc::new(
            crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
                self.test_presenter().unwrap(),
            ),
        );
        terminal.connect_state(presenter.clone()).unwrap();
        let subscriptions =
            crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase::new(
                presenter.clone(),
                Some(terminal),
                crate::test_support::state_subscription::terminal_driver(),
            );

        TerminalSubscriptions {
            usecase: self,
            terminal: subscriptions,
            presenter,
        }
    }
}

impl TerminalSubscriptions {
    pub(crate) fn with_terminal(
        self,
        terminal: Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    ) -> Self {
        self.usecase.with_terminal(terminal)
    }

    pub(crate) fn schedule_terminal_refresh(
        &self,
        clients: Vec<String>,
        target: crate::usecase::state_subscription::SubscriptionTarget,
    ) {
        self.terminal.schedule_terminal_refresh(clients, target);
    }
    pub(crate) fn test_worker_count(&self) -> usize {
        self.terminal.test_worker_count() + self.usecase.test_worker_count()
    }
    pub(crate) fn deps(&self) -> crate::adaptor::controller::api::StateSubscriptionDeps {
        crate::adaptor::controller::api::StateSubscriptionDeps::new(
            self.usecase.clone(),
            Arc::new(self.usecase.test_presenter().unwrap().clone()),
            self.terminal.clone(),
        )
    }
    pub(crate) fn test_presenter(
        &self,
    ) -> Option<&crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter>
    {
        Some(&self.presenter)
    }
    pub(crate) fn open(
        &self,
        id: String,
    ) -> Result<impl Stream<Item = StateSubscriptionEvent> + Send + use<>, SubscriptionError> {
        self.deps().stream(id)
    }
    pub(crate) fn close_client(&self, id: &str) {
        self.usecase.close_client(id);
        self.terminal.close_client(id);
    }
}

pub(crate) fn terminal_application_fixture() -> (
    Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    Arc<crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor>,
    Arc<crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub>,
    crate::domain::terminal_surface::entities::TerminalSurface,
){
    use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
    use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
    let hub =
        Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
    let gateway = Arc::new(crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}), std::path::PathBuf::new(), hub.clone(), false,
    ));
    let owner = crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
        crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
    )
    .unwrap();
    let surface = crate::domain::terminal_surface::entities::TerminalSurface::new(1, owner, None);
    gateway.insert_surface(surface.clone());
    hub.initialize(registration(&surface.session_key, "/repo", None, 1, 0))
        .unwrap();
    let terminal = terminal_application_with_gateway(gateway.clone(), hub.clone());
    (terminal, gateway, hub, surface)
}

pub(crate) fn terminal_application_with_gateway(
    gateway: Arc<
        dyn crate::domain::terminal_surface::gateway::TerminalSurfaceGateway + Send + Sync,
    >,
    hub: Arc<crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub>,
) -> Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication> {
    Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
        Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway,
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
    ))
}

thread_local! {
    static IDLE_DRIVER_RECEIVERS: std::cell::RefCell<Vec<Box<dyn std::any::Any>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

fn driver<T: 'static>(
    start: impl FnOnce() -> tokio::sync::mpsc::UnboundedSender<T>,
) -> tokio::sync::mpsc::UnboundedSender<T> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return start();
    }
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<T>();
    IDLE_DRIVER_RECEIVERS.with(|receivers| receivers.borrow_mut().push(Box::new(receiver)));
    sender
}

pub(crate) fn read_driver(
) -> tokio::sync::mpsc::UnboundedSender<crate::usecase::state_subscription::ReadWorker> {
    driver(|| {
        crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
            let period = crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration();
            Box::pin(crate::infrastructure::timer::ticks_after(period, period))
        }))
    })
}

pub(crate) fn pending_read_driver(
) -> tokio::sync::mpsc::UnboundedSender<crate::usecase::state_subscription::ReadWorker> {
    driver(|| {
        crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
            Box::pin(futures_util::stream::pending())
        }))
    })
}

pub(crate) fn scan_driver(
    duration: std::time::Duration,
) -> tokio::sync::mpsc::UnboundedSender<crate::usecase::repository_state::runtime::ScanWorker> {
    crate::adaptor::controller::repository_scan::start(
        crate::usecase::retry::shared().clone(),
        Arc::new(crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime),
        crate::infrastructure::timer::delays(duration),
    )
}

pub(crate) fn terminal_driver() -> tokio::sync::mpsc::UnboundedSender<
    crate::usecase::terminal_surface::subscription::TerminalRefresh,
> {
    driver(crate::adaptor::controller::terminal_subscription::start)
}

pub(crate) fn repository_driver(
) -> tokio::sync::mpsc::UnboundedSender<crate::usecase::repository_state::runtime::ScanWorker> {
    driver(|| {
        crate::adaptor::controller::repository_scan::start(
        crate::usecase::retry::shared().clone(),
        Arc::new(crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime),
        Arc::new(|| Box::pin(async {})),
    )
    })
}

#[cfg(test)]
#[path = "state_subscription_test.rs"]
mod state_subscription_tests;
