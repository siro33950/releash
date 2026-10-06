use std::sync::Arc;

use axum::Router;

use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect::command_error;
use crate::adaptor::presenter::connect_wire::{rpc, to_rpc, to_wire};

#[derive(Clone)]
pub struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    state_subscriptions: Option<StateSubscriptionDeps>,
    priority: super::client_priority::PriorityInterceptor,
    daemon: crate::usecase::daemon::DaemonUsecase,
    provider_lifecycle: Option<(
        Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>,
        Arc<dyn crate::domain::provider_lifecycle::ProviderPayloadInterpreter>,
    )>,
}

#[derive(Clone)]
pub struct StateSubscriptionDeps {
    usecase: crate::usecase::state_subscription::StateSubscriptionUsecase,
    presenter: Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter>,
    terminal: crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase,
}

impl StateSubscriptionDeps {
    pub(crate) fn new(
        usecase: crate::usecase::state_subscription::StateSubscriptionUsecase,
        presenter: Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter>,
        terminal: crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase,
    ) -> Self {
        Self {
            usecase,
            presenter,
            terminal,
        }
    }
}

impl ClientApiDeps {
    pub(crate) fn new(
        dispatch: Arc<ClientCommandDispatch>,
        priority: super::client_priority::PriorityInterceptor,
    ) -> Self {
        Self {
            dispatch: dispatch.clone(),
            state_subscriptions: None,
            provider_lifecycle: None,
            priority,
            daemon: dispatch.daemon.clone(),
        }
    }

    pub fn with_provider_lifecycle(
        mut self,
        ingress: Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>,
        interpreter: Arc<dyn crate::domain::provider_lifecycle::ProviderPayloadInterpreter>,
    ) -> Self {
        self.provider_lifecycle = Some((ingress, interpreter));
        self
    }
    pub fn with_state_subscriptions(mut self, subscriptions: StateSubscriptionDeps) -> Self {
        self.state_subscriptions = Some(subscriptions);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn priority_limits(&self) -> &crate::common::concurrency::PriorityLimits {
        self.priority.gate.limits()
    }

    fn subscriptions(&self) -> Result<&StateSubscriptionDeps, connectrpc::ConnectError> {
        self.state_subscriptions.as_ref().ok_or_else(|| {
            crate::adaptor::presenter::connect::classified_error(
                crate::adaptor::presenter::error::AppError::unavailable(
                    "State subscriptions unavailable",
                ),
            )
        })
    }

    pub async fn execute(
        &self,
        deadline: Option<std::time::Instant>,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, connectrpc::ConnectError> {
        ingress(deadline, self.execute_scoped(command)).await
    }

    async fn execute_scoped(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, connectrpc::ConnectError> {
        let dispatch = self.dispatch.clone();
        crate::common::operation_context::spawned(async move {
            dispatch
                .dispatch_admitted(command)
                .await
                .map_err(command_error)
        })
        .await
        .map_err(task_error)?
        .map_err(|error| {
            crate::adaptor::presenter::connect::classified_error(
                crate::adaptor::presenter::error::AppError::from_failure(
                    crate::domain::failure::TechnicalFailure::from(error),
                ),
            )
        })?
    }
}

pub async fn ingress<T>(
    deadline: Option<std::time::Instant>,
    operation: impl std::future::Future<Output = Result<T, connectrpc::ConnectError>>,
) -> Result<T, connectrpc::ConnectError> {
    crate::common::operation_context::ingress(deadline, operation)
        .await
        .map_err(|error| {
            crate::adaptor::presenter::connect::classified_error(
                crate::adaptor::presenter::error::AppError::from_failure(
                    crate::domain::failure::TechnicalFailure::from(error),
                ),
            )
        })?
}

pub fn task_error(error: tokio::task::JoinError) -> connectrpc::ConnectError {
    crate::adaptor::presenter::connect::classified_error(
        crate::domain::failure::TechnicalFailure::from(error),
    )
}

pub fn router(deps: Option<ClientApiDeps>, default_timeout: std::time::Duration) -> Router {
    let Some(deps) = deps else {
        return Router::new();
    };
    let priority = deps.priority.clone();
    let admission = super::client_admission::DaemonAdmission(deps.daemon.clone());
    let service = connectrpc::Router::new()
        .add_service(Arc::new(deps))
        .into_axum_service()
        .with_interceptor(priority)
        .with_interceptor(admission)
        .with_deadline_policy(
            connectrpc::DeadlinePolicy::new().with_default_timeout(default_timeout),
        )
        .with_limits(
            connectrpc::Limits::default()
                .with_max_request_body_size(16 * 1024 * 1024)
                .with_max_message_size(16 * 1024 * 1024),
        );
    Router::new().route_service("/releash.client.v1.ClientService/{method}", service)
}

include!(concat!(env!("OUT_DIR"), "/client_service.rs"));

pub struct StateStreamPermit {
    subscriptions: StateSubscriptionDeps,
    id: String,
}

impl Drop for StateStreamPermit {
    fn drop(&mut self) {
        self.subscriptions.usecase.close_client(&self.id);
        self.subscriptions.terminal.close_client(&self.id);
        self.subscriptions
            .usecase
            .with_active_targets(|active| self.subscriptions.presenter.close(&self.id, active));
    }
}

impl StateSubscriptionDeps {
    pub fn open_stream(
        &self,
        id: String,
    ) -> Result<StateStreamPermit, crate::usecase::state_subscription::SubscriptionError> {
        self.usecase.open_client(id.clone())?;
        if let Err(error) = self.terminal.open_client(id.clone()) {
            self.usecase.close_client(&id);
            return Err(error);
        }
        if let Err(error) = self.presenter.open(id.clone()) {
            self.usecase.close_client(&id);
            self.terminal.close_client(&id);
            return Err(error);
        }
        Ok(StateStreamPermit {
            subscriptions: self.clone(),
            id,
        })
    }

    pub fn stream(
        &self,
        id: String,
    ) -> Result<
        impl futures_util::Stream<
                Item = crate::adaptor::presenter::state_subscription::StateSubscriptionEvent,
            > + Send
            + use<>,
        crate::usecase::state_subscription::SubscriptionError,
    > {
        let permit = self.open_stream(id.clone())?;
        let terminal = self.terminal.clone();
        Ok(self.presenter.stream(id, permit, move |raw, clients| {
            if let Ok(target) = crate::usecase::state_subscription::SubscriptionTarget::parse(&raw)
            {
                terminal.schedule_terminal_refresh(clients, target);
            }
        }))
    }

    fn stream_wire(
        &self,
        id: String,
    ) -> Result<
        impl futures_util::Stream<
                Item = Result<rpc::StateSubscriptionEvent, connectrpc::ConnectError>,
            > + Send
            + use<>,
        crate::usecase::state_subscription::SubscriptionError,
    > {
        use futures_util::StreamExt;
        Ok(self
            .stream(id)?
            .map(crate::adaptor::presenter::state_subscription_wire::event))
    }

    pub async fn start_subscription(
        &self,
        client: &str,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
        id: &str,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), crate::usecase::state_subscription::StateReadError> {
        use crate::usecase::state_subscription::{StateReadError, SubscriptionTarget};
        let delivery = self
            .presenter
            .reserve_delivery(client, id, &target.to_string(), cursor)
            .map_err(StateReadError::from_error)?;
        match target {
            SubscriptionTarget::Terminal(_) => {
                self.terminal
                    .start_subscription(client, target, id, &delivery)
                    .await
            }
            _ => {
                self.usecase
                    .start_subscription(client, target, &delivery)
                    .await
            }
        }
    }

    pub async fn stop_subscription(
        &self,
        id: &str,
    ) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
        let Some((client, raw, delivery)) = self.presenter.delivery(id) else {
            return Ok(());
        };
        let target = crate::usecase::state_subscription::SubscriptionTarget::parse(&raw)?;
        match target {
            crate::usecase::state_subscription::SubscriptionTarget::Terminal(_) => {
                self.terminal.stop_delivery(&client, &target, id, &delivery)
            }
            _ => {
                self.usecase
                    .stop_subscription(&client, &target, &delivery)
                    .await
            }
        }
    }

    pub fn terminal_processed(
        &self,
        id: &str,
        units: usize,
    ) -> Result<(), crate::usecase::state_subscription::StateReadError> {
        self.terminal.terminal_processed(id, units)
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;

#[cfg(feature = "test-support")]
impl StateSubscriptionDeps {
    pub fn test_presenter(
        &self,
    ) -> Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter> {
        self.presenter.clone()
    }
    pub fn test_usecase(&self) -> &crate::usecase::state_subscription::StateSubscriptionUsecase {
        &self.usecase
    }
}
