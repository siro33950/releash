use crate::adaptor::protocol::{
    client as wire,
    connect::{rpc, to_rpc, to_wire},
};
use crate::domain::state_subscription::{
    connection::{ReceivedState, StateClientError, StateClientGateway, StateConnection},
    StateValue, Version,
};
use crate::usecase::client_connection::ClientConnectionQueryService;
use futures_util::{Stream, StreamExt};
use std::{pin::Pin, sync::Arc};

pub(crate) struct StateClientGatewayImpl(pub Arc<dyn ClientConnectionQueryService>);
struct Connection {
    client: rpc::ClientServiceClient<connectrpc::client::HttpClient>,
    id: String,
    events: Pin<Box<dyn Stream<Item = Result<ReceivedState, StateClientError>> + Send>>,
}

#[async_trait::async_trait]
impl StateConnection for Connection {
    async fn start(
        &mut self,
        target: &str,
        version: Option<&Version>,
    ) -> Result<(), StateClientError> {
        self.client
            .start_state_subscription(
                to_rpc::<rpc::StartStateSubscriptionRequest>(
                    &wire::StartStateSubscriptionRequest {
                        client_id: self.id.clone(),
                        target: target.into(),
                        version: version.map(|v| wire::StateVersion {
                            epoch: v.epoch.clone(),
                            sequence: v.sequence,
                        }),
                    },
                )
                .map_err(error)?,
            )
            .await
            .map_err(error)?;
        Ok(())
    }
    async fn stop(&mut self, target: &str) -> Result<(), StateClientError> {
        self.client
            .stop_state_subscription(
                to_rpc::<rpc::StopStateSubscriptionRequest>(&wire::StopStateSubscriptionRequest {
                    client_id: self.id.clone(),
                    target: target.into(),
                })
                .map_err(error)?,
            )
            .await
            .map_err(error)?;
        Ok(())
    }
    async fn receive(&mut self) -> Result<ReceivedState, StateClientError> {
        self.events
            .next()
            .await
            .ok_or_else(|| StateClientError("State stream ended".into()))?
    }
}

fn error(error: impl std::fmt::Display) -> StateClientError {
    StateClientError(error.to_string())
}

#[async_trait::async_trait]
impl StateClientGateway for StateClientGatewayImpl {
    async fn connect(&self) -> Result<Box<dyn StateConnection>, StateClientError> {
        let endpoint = self.0.read().map_err(error)?;
        let config = connectrpc::client::ClientConfig::new(endpoint.url.parse().map_err(error)?)
            .with_default_header("authorization", format!("Bearer {}", endpoint.token))
            .with_default_header("origin", "tauri://localhost");
        let client =
            rpc::ClientServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
        let id = uuid::Uuid::new_v4().to_string();
        let mut stream = client
            .open_state_stream(
                to_rpc::<rpc::OpenStateStreamRequest>(&wire::OpenStateStreamRequest {
                    client_id: id.clone(),
                })
                .map_err(error)?,
            )
            .await
            .map_err(error)?;
        let ready = stream
            .message::<rpc::StateSubscriptionEvent>()
            .await
            .map_err(error)?
            .ok_or_else(|| StateClientError("State stream closed before ready".into()))?;
        let ready: wire::StateSubscriptionEvent =
            to_wire(&ready.to_owned_message()).map_err(error)?;
        if !matches!(
            ready.event,
            Some(wire::state_subscription_event::Event::Ready(_))
        ) {
            return Err(StateClientError("Expected state stream ready".into()));
        }
        let events = futures_util::stream::unfold(stream, |mut stream| async move {
            let result = match stream.message::<rpc::StateSubscriptionEvent>().await {
                Ok(Some(message)) => {
                    to_wire::<wire::StateSubscriptionEvent>(&message.to_owned_message())
                        .map_err(error)
                        .and_then(decode)
                }
                Ok(None) => return None,
                Err(cause) => Err(error(cause)),
            };
            Some((result, stream))
        });
        Ok(Box::new(Connection {
            client,
            id,
            events: Box::pin(events),
        }))
    }
}

fn decode(event: wire::StateSubscriptionEvent) -> Result<ReceivedState, StateClientError> {
    use wire::state_subscription_event::Event;
    if event.target.is_empty() {
        return Err(StateClientError("Unexpected state target".into()));
    }
    let version = event
        .version
        .ok_or_else(|| StateClientError("Missing state version".into()))?;
    let (snapshot, payload) = match event.event {
        Some(Event::Snapshot(payload)) => (true, Some(payload)),
        Some(Event::Change(change)) if !change.delta => (
            false,
            Some(
                change
                    .payload
                    .ok_or_else(|| StateClientError("Missing change payload".into()))?,
            ),
        ),
        Some(Event::Bookmark(_)) => (false, None),
        _ => return Err(StateClientError("Unexpected state event".into())),
    };
    let value = payload
        .map(|payload| match payload.value {
            Some(wire::state_payload::Value::RepositoryPaths(paths)) => {
                Ok(StateValue::RepositoryPaths(paths.items))
            }
            None => Err(StateClientError("Missing state payload".into())),
        })
        .transpose()?;
    Ok(ReceivedState {
        target: event.target,
        version: Version {
            epoch: version.epoch,
            sequence: version.sequence,
        },
        snapshot,
        value,
    })
}

#[cfg(test)]
#[path = "state_client_test.rs"]
mod state_client_tests;
