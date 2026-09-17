use std::{sync::Weak, time::Duration};

use super::protocol::client::{self as wire, envelope::Body};
use crate::domain::client_operation::{
    policy,
    registry::{OperationDecision, OperationIdentity, OperationReference},
};
use crate::usecase::client_operation::ClientOperationUsecase;

pub(super) async fn maintain<R: Clone + Send + 'static>(
    operations: Weak<ClientOperationUsecase<R>>,
) {
    loop {
        tokio::time::sleep(Duration::from_millis(policy::TICK_INTERVAL_MS)).await;
        let Some(operations) = operations.upgrade() else {
            break;
        };
        operations.maintain().await;
    }
}

pub(super) fn identity(
    request: &wire::CommandRequest,
) -> Result<OperationIdentity, wire::CommandError> {
    let (command, args) = request
        .clone()
        .into_value()
        .map_err(super::client_stream::invalid)?;
    Ok(crate::adaptor::gateway::shared::client_operation::identity(
        command, args,
    ))
}

pub(super) fn input(
    request: &wire::CommandRequest,
) -> Result<(OperationIdentity, Vec<OperationReference>), wire::CommandError> {
    let references = references(&request.predecessors)?;
    Ok((identity(request)?, references))
}

pub(super) fn references(
    items: &[wire::OperationReference],
) -> Result<Vec<OperationReference>, wire::CommandError> {
    fn digest(bytes: &[u8]) -> Result<Option<[u8; 32]>, wire::CommandError> {
        if bytes.is_empty() {
            return Ok(None);
        }
        bytes
            .try_into()
            .map(Some)
            .map_err(|_| super::client_stream::invalid("Invalid operation identity"))
    }
    items
        .iter()
        .map(|previous| {
            Ok(OperationReference {
                id: previous.request_id.clone(),
                command: previous.command.clone(),
                uncertain: previous.uncertain,
                fingerprint: digest(&previous.fingerprint)?,
                target: digest(&previous.ordering_target)?,
            })
        })
        .collect::<Result<Vec<_>, wire::CommandError>>()
}

pub(super) fn response(id: &str, state: OperationDecision<wire::Envelope>) -> wire::Envelope {
    use OperationDecision::*;
    let (state, operation_id) = match state {
        Completed(result) => return result,
        Conflict | Full => {
            return wire::response(
                id.into(),
                Err(crate::other::AppError::coded(
                    if matches!(state, Conflict) {
                        "OPERATION_CONFLICT"
                    } else {
                        "OPERATION_LIMIT"
                    },
                    if matches!(state, Conflict) {
                        "Operation ID is bound to different arguments"
                    } else {
                        "Confirm outstanding operations before starting another operation"
                    },
                )
                .into()),
            )
        }
        Pending => ("pending", String::new()),
        NotSent => ("not_sent", String::new()),
        WatchReleased => ("watch_released", String::new()),
        Disconnected => ("disconnected", String::new()),
        Unknown => ("unknown", String::new()),
        Ready => ("ready", String::new()),
        Blocked => ("blocked", String::new()),
        Bound(original) => ("bound", original),
    };
    wire::Envelope {
        body: Some(Body::OperationStatus(wire::OperationStatus {
            request_id: id.into(),
            state: state.into(),
            operation_id,
            ..Default::default()
        })),
    }
}

pub(super) fn decision(
    id: &str,
    state: OperationDecision<wire::Envelope>,
    identity: &OperationIdentity,
) -> wire::Envelope {
    let mut envelope = response(id, state);
    if let Some(Body::OperationStatus(status)) = &mut envelope.body {
        status.fingerprint = identity.fingerprint.to_vec();
        status.ordering_target = identity
            .target
            .map_or_else(Vec::new, |(_, target)| target.to_vec());
    }
    envelope
}

#[cfg(test)]
#[path = "client_operation_test.rs"]
mod client_operation_tests;

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_millis() as u64
}
