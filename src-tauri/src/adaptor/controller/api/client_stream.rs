use connectrpc::{ConnectError, ServiceStream};
use futures_util::StreamExt;
use std::sync::Arc;

use super::protocol::client as wire;
use super::protocol::connect::{command_error, command_error_with_code, rpc, to_rpc};
use crate::adaptor::controller::client::{convert, required};
use crate::adaptor::controller::terminal_surface::{
    invalid_owner_error, TerminalCommandError, TerminalCommandOperation,
};
use crate::adaptor::protocol::terminal::{TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1};
use crate::domain::terminal_surface::subscriptions::TerminalSubscriptionError;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use crate::usecase::terminal_surface::subscriptions::{
    TerminalSubscriptionEvent, TerminalSubscriptionUsecase, UsecaseError,
};

#[derive(Clone)]
pub(crate) struct TerminalApiDeps {
    usecase: TerminalSubscriptionUsecase,
}
impl TerminalApiDeps {
    pub(crate) fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self {
            usecase: TerminalSubscriptionUsecase::new(application),
        }
    }
    pub(crate) fn detach(&self, id: &str) {
        self.usecase.detach(id);
    }
    pub(crate) fn subscribe(
        &self,
        id: String,
    ) -> Result<ServiceStream<rpc::TerminalSubscriptionEvent>, ConnectError> {
        validate_identifier(&id)?;
        let stream = self.usecase.subscribe(id).map_err(subscription_error)?;
        Ok(Box::pin(stream.map(|event| {
            use wire::terminal_subscription_event::Event;
            let (attachment_id, stream_id, event) = match event {
                TerminalSubscriptionEvent::Ready => {
                    (String::new(), String::new(), Event::Ready(wire::Unit {}))
                }
                TerminalSubscriptionEvent::Item {
                    attachment_id,
                    stream_id,
                    item,
                } => (
                    attachment_id,
                    stream_id,
                    Event::Item(TerminalSurfaceStreamItemV1::from(item).into()),
                ),
                TerminalSubscriptionEvent::Closed {
                    attachment_id,
                    stream_id,
                    resynchronize,
                } => (
                    attachment_id,
                    stream_id,
                    Event::Closed(wire::TerminalStreamClosed { resynchronize }),
                ),
            };
            to_rpc(&wire::TerminalSubscriptionEvent {
                attachment_id,
                stream_id,
                event: Some(event),
            })
        })))
    }
    pub(crate) fn attach(
        &self,
        subscription_id: &str,
        stream_id: String,
        args: wire::AttachTerminalSurfaceRequest,
    ) -> Result<(), ConnectError> {
        validate_identifier(subscription_id)?;
        validate_identifier(&stream_id)?;
        let id = required(args.attachment_id, "attachmentId").map_err(command_error)?;
        let owner: TerminalSurfaceOwnerV1 =
            convert(required(args.owner, "owner").map_err(command_error)?)
                .map_err(command_error)?;
        let recovery = required(args.recovery, "recovery").map_err(command_error)?;
        if id.is_empty() || id.len() > 128 {
            return Err(command_error(wire::CommandError::from(
                TerminalCommandError {
                    code: "INVALID_REQUEST".into(),
                    message: "Invalid attachment ID".into(),
                },
            )));
        }
        let operation = TerminalCommandOperation::attachment(recovery);
        let owner = owner
            .try_into()
            .map_err(|cause| command_error(invalid_owner_error(operation, cause).into()))?;
        self.usecase
            .attach(subscription_id, id, stream_id, &owner)
            .map_err(|error| match error {
                UsecaseError::Application(error) => {
                    command_error(TerminalCommandError::from_usecase(error, operation).into())
                }
                UsecaseError::Subscription(error) => subscription_error(error),
            })
    }
}
pub(super) fn validate_identifier(id: &str) -> Result<(), ConnectError> {
    if id.len() > 128 {
        return Err(ConnectError::invalid_argument(
            "Identifier exceeds 128 bytes",
        ));
    }
    Ok(())
}

fn subscription_error(error: TerminalSubscriptionError) -> ConnectError {
    match error {
        TerminalSubscriptionError::InvalidId => ConnectError::invalid_argument(error.to_string()),
        TerminalSubscriptionError::AlreadyExists => ConnectError::already_exists(error.to_string()),
        TerminalSubscriptionError::Ended => ConnectError::not_found(error.to_string()),
        TerminalSubscriptionError::Limit | TerminalSubscriptionError::PendingLimit => {
            ConnectError::resource_exhausted(error.to_string())
        }
        TerminalSubscriptionError::AttachmentLimit => command_error_with_code(
            crate::other::AppError::coded("TERMINAL_ATTACHMENT_LIMIT", error.to_string()).into(),
            connectrpc::ErrorCode::ResourceExhausted,
        ),
    }
}

#[cfg(all(test, feature = "desktop"))]
#[path = "client_stream_test.rs"]
mod client_stream_tests;
