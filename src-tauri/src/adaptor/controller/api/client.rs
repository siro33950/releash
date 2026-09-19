use std::sync::Arc;

use axum::Router;

use super::client_stream::TerminalApiDeps;
use super::protocol::client as wire;
use super::protocol::connect::{command_error, command_error_with_code, rpc, to_rpc, to_wire};
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::gateway::push::{ClientPushError, ClientPushGateway};

struct EncodedPush(Arc<[u8]>);

impl connectrpc::Encodable<rpc::Push> for EncodedPush {
    fn encode(
        &self,
        codec: connectrpc::CodecFormat,
    ) -> Result<axum::body::Bytes, connectrpc::ConnectError> {
        match codec {
            connectrpc::CodecFormat::Proto => Ok(axum::body::Bytes::from_owner(self.0.clone())),
            _ => {
                let push = <rpc::Push as buffa::Message>::decode_from_slice(&self.0)
                    .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?;
                push.encode(codec)
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    push: ClientPushGateway,
    terminal: Option<TerminalApiDeps>,
    desktop_settings: Option<Arc<crate::usecase::app_config::AppConfigUsecase>>,
    request_limit: Arc<tokio::sync::Semaphore>,
    watcher: Arc<crate::usecase::watcher::WatcherUsecase>,
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
            terminal: None,
            desktop_settings: None,
            request_limit: Arc::new(tokio::sync::Semaphore::new(64)),
            watcher,
        }
    }

    pub(crate) fn with_desktop_settings(
        mut self,
        settings: crate::usecase::app_config::AppConfigUsecase,
    ) -> Self {
        self.desktop_settings = Some(Arc::new(settings));
        self
    }

    fn desktop_settings(&self) -> Result<Option<wire::DesktopSettings>, String> {
        self.desktop_settings
            .as_ref()
            .map(|settings| {
                settings
                    .desktop_settings()
                    .map(Into::into)
                    .map_err(String::from)
            })
            .transpose()
    }

    pub(super) fn with_terminal(mut self, terminal: Option<TerminalApiDeps>) -> Self {
        self.terminal = terminal;
        self
    }

    fn request_permit(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, connectrpc::ConnectError> {
        self.request_limit.clone().try_acquire_owned().map_err(|_| {
            let message = "Too many pending client commands";
            let mut error = command_error_with_code(
                crate::other::AppError::coded("CLIENT_REQUEST_LIMIT", message).into(),
                connectrpc::ErrorCode::ResourceExhausted,
            );
            error.message = Some(message.into());
            error
        })
    }

    async fn execute(
        &self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, connectrpc::ConnectError> {
        let permit = self.request_permit()?;
        self.dispatch.admit(command.name()).map_err(command_error)?;
        if let wire::command_request::Command::StopWatching(ref args) = command {
            let id = crate::adaptor::controller::client::required(args.watcher_id, "watcherId")
                .map_err(command_error)?;
            let watcher = self.watcher.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                watcher.stop(id)
            })
            .await
            .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?
            .map_err(watch_error)?;
            return Ok(wire::command_result::Command::StopWatching(wire::Unit {}));
        }

        if let wire::command_request::Command::DetachTerminalSurface(args) = command {
            let id =
                crate::adaptor::controller::client::required(args.attachment_id, "attachmentId")
                    .map_err(command_error)?;
            if let Some(terminal) = &self.terminal {
                terminal.detach(&id);
            }
            return Ok(wire::command_result::Command::DetachTerminalSurface(
                wire::Unit {},
            ));
        }
        let dispatch = self.dispatch.clone();
        tokio::spawn(async move {
            let _permit = permit;
            dispatch
                .dispatch_admitted(command)
                .await
                .map_err(command_error)
        })
        .await
        .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?
    }

    async fn watch(
        &self,
        subscription_id: String,
        path: String,
        git: bool,
    ) -> Result<rpc::ResultUint64, connectrpc::ConnectError> {
        let permit = self.request_permit()?;
        self.dispatch
            .admit(if git {
                "start_git_dir_watching"
            } else {
                "start_watching"
            })
            .map_err(command_error)?;
        let watcher = self.watcher.clone();
        let id = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            watcher.watch(&subscription_id, &path, git)
        })
        .await
        .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?
        .map_err(watch_error)?;
        to_rpc(&wire::ResultUint64 { value: Some(id) })
    }
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

fn watch_error(error: crate::usecase::watcher::UsecaseError) -> connectrpc::ConnectError {
    use crate::domain::repository::watch_subscriptions::WatchSubscriptionError;
    use crate::usecase::watcher::UsecaseError;
    match error {
        UsecaseError::Subscription(WatchSubscriptionError::NotFound) => {
            connectrpc::ConnectError::not_found(error.to_string())
        }
        UsecaseError::Subscription(WatchSubscriptionError::AlreadyExists) => {
            connectrpc::ConnectError::already_exists(error.to_string())
        }
        UsecaseError::Subscription(WatchSubscriptionError::Limit) => {
            connectrpc::ConnectError::resource_exhausted(error.to_string())
        }
        error => command_error(wire::CommandError::from(error.to_string())),
    }
}

pub(crate) fn router(deps: Option<ClientApiDeps>) -> Router {
    let Some(deps) = deps else {
        return Router::new();
    };
    let service = connectrpc::Router::new()
        .add_service(Arc::new(deps))
        .into_axum_service()
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
