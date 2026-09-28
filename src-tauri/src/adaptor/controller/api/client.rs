use std::sync::Arc;

use axum::Router;

use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::gateway::push::{ClientPushError, ClientPushGateway};
use crate::adaptor::presenter::client as wire;
use crate::adaptor::presenter::connect::command_error;
use crate::adaptor::presenter::connect_wire::{rpc, to_rpc, to_wire};
use crate::adaptor::presenter::push::EncodedPush;

#[derive(Clone)]
pub(crate) struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    push: ClientPushGateway,
    state_subscriptions: Option<StateSubscriptionDeps>,
    desktop_settings: Option<Arc<crate::usecase::app_config::AppConfigUsecase>>,
    request_limit: Arc<tokio::sync::Semaphore>,
    watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
}

#[derive(Clone)]
pub(crate) struct StateSubscriptionDeps {
    usecase: crate::usecase::state_subscription::StateSubscriptionUsecase,
    presenter: Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter>,
}

impl StateSubscriptionDeps {
    pub(crate) fn new(
        usecase: crate::usecase::state_subscription::StateSubscriptionUsecase,
        presenter: Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter>,
    ) -> Self {
        Self { usecase, presenter }
    }
}

impl ClientApiDeps {
    pub(crate) fn new(
        dispatch: Arc<ClientCommandDispatch>,
        push: ClientPushGateway,
        watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
    ) -> Self {
        Self {
            dispatch,
            push,
            state_subscriptions: None,
            desktop_settings: None,
            request_limit: Arc::new(tokio::sync::Semaphore::new(64)),
            watcher,
        }
    }

    pub(crate) fn with_state_subscriptions(mut self, subscriptions: StateSubscriptionDeps) -> Self {
        self.state_subscriptions = Some(subscriptions);
        self
    }

    fn state_presenter(
        &self,
    ) -> Result<
        &Arc<crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter>,
        connectrpc::ConnectError,
    > {
        self.state_subscriptions
            .as_ref()
            .map(|deps| &deps.presenter)
            .ok_or_else(|| {
                crate::adaptor::presenter::connect::classified_error(
                    crate::adaptor::presenter::error::AppError::unavailable(
                        "State subscriptions unavailable",
                    ),
                )
            })
    }

    fn state_subscriptions(
        &self,
    ) -> Result<
        &crate::usecase::state_subscription::StateSubscriptionUsecase,
        connectrpc::ConnectError,
    > {
        self.state_subscriptions
            .as_ref()
            .map(|deps| &deps.usecase)
            .ok_or_else(|| {
                crate::adaptor::presenter::connect::classified_error(
                    crate::adaptor::presenter::error::AppError::unavailable(
                        "State subscriptions unavailable",
                    ),
                )
            })
    }

    pub(crate) fn with_desktop_settings(
        mut self,
        settings: crate::usecase::app_config::AppConfigUsecase,
    ) -> Self {
        self.desktop_settings = Some(Arc::new(settings));
        self
    }

    fn desktop_settings(
        &self,
    ) -> Result<Option<wire::DesktopSettings>, crate::usecase::app_config::error::UsecaseError>
    {
        self.desktop_settings
            .as_ref()
            .map(|settings| settings.desktop_settings().map(Into::into))
            .transpose()
    }

    fn request_permit(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, connectrpc::ConnectError> {
        self.request_limit
            .clone()
            .try_acquire_owned()
            .map_err(|_| crate::adaptor::presenter::connect::request_capacity_error())
    }

    async fn execute(
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
        let _permit = self.request_permit()?;
        self.dispatch.admit(command.name()).map_err(command_error)?;
        if let wire::command_request::Command::StopWatching(ref args) = command {
            let id = crate::adaptor::controller::client::required(args.watcher_id, "watcherId")
                .map_err(command_error)?;
            let watcher = self.watcher.clone();
            crate::common::operation_context::spawn_blocking(move || watcher.stop(id))
                .await
                .map_err(task_error)?
                .map_err(watch_error)?;
            return Ok(wire::command_result::Command::StopWatching(wire::Unit {}));
        }

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

    async fn watch(
        &self,
        deadline: Option<std::time::Instant>,
        subscription_id: String,
        path: String,
        git: bool,
    ) -> Result<rpc::ResultUint64, connectrpc::ConnectError> {
        ingress(deadline, self.watch_scoped(subscription_id, path, git)).await
    }

    async fn watch_scoped(
        &self,
        subscription_id: String,
        path: String,
        git: bool,
    ) -> Result<rpc::ResultUint64, connectrpc::ConnectError> {
        let _permit = self.request_permit()?;
        self.dispatch
            .admit(if git {
                "start_git_dir_watching"
            } else {
                "start_watching"
            })
            .map_err(command_error)?;
        let watcher = self.watcher.clone();
        let context = crate::common::operation_context::current();
        let result = crate::common::operation_context::scope(context.clone(), async {
            crate::common::operation_context::spawn_blocking(move || {
                context.check(std::time::Instant::now()).map_err(|error| {
                    command_error(
                        crate::adaptor::presenter::error::AppError::from_failure(
                            crate::domain::failure::TechnicalFailure::from(error),
                        )
                        .into(),
                    )
                })?;
                let result = watcher.watch(&subscription_id, &path, git);
                if let Err(error) = context.check(std::time::Instant::now()) {
                    if let Ok(id) = result {
                        watcher.release(id);
                    }
                    return Err(command_error(
                        crate::adaptor::presenter::error::AppError::from_failure(
                            crate::domain::failure::TechnicalFailure::from(error),
                        )
                        .into(),
                    ));
                }
                result.map_err(watch_error)
            })
            .await
        })
        .await
        .map_err(task_error)?;
        let id = result?;
        wire::watch_id_to_rpc(id)
    }
}

async fn ingress<T>(
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

fn response_headers(command: &wire::command_result::Command) -> axum::http::HeaderMap {
    use wire::command_result::Command;
    let mut headers = axum::http::HeaderMap::new();
    if matches!(
        command,
        Command::UpdateAppSettings(_)
            | Command::UpdateCrashReporting(_)
            | Command::UpdatePerformanceTelemetry(_)
    ) {
        headers.insert("releash-desktop-settings-changed", "true".parse().unwrap());
    }
    headers
}

fn task_error(error: tokio::task::JoinError) -> connectrpc::ConnectError {
    crate::adaptor::presenter::connect::classified_error(
        crate::domain::failure::TechnicalFailure::from(error),
    )
}

fn watch_error(error: crate::usecase::watcher::UsecaseError) -> connectrpc::ConnectError {
    command_error(crate::adaptor::presenter::error::AppError::from_failure(error).into())
}

pub(crate) fn router(deps: Option<ClientApiDeps>) -> Router {
    let Some(deps) = deps else {
        return Router::new();
    };
    let service = connectrpc::Router::new()
        .add_service(Arc::new(deps))
        .into_axum_service()
        .with_deadline_policy(
            connectrpc::DeadlinePolicy::new()
                .with_default_timeout(std::time::Duration::from_secs(120)),
        )
        .with_limits(
            connectrpc::Limits::default()
                .with_max_request_body_size(16 * 1024 * 1024)
                .with_max_message_size(16 * 1024 * 1024),
        );
    Router::new().route_service("/releash.client.v1.ClientService/{method}", service)
}

include!(concat!(env!("OUT_DIR"), "/client_service.rs"));

#[cfg(all(test, feature = "desktop"))]
#[path = "client_test.rs"]
mod client_tests;
