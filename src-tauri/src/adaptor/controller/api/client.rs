use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Router};
use futures_util::{SinkExt, StreamExt};

use crate::adaptor::controller::command::client::ClientCommandDispatch;
use crate::adaptor::gateway::push::{ClientPushGateway, ClientPushSubscription};
use crate::adaptor::protocol::client::CLIENT_WS_PATH;
use crate::other::AppError;

use super::protocol::{ClientRequest, CommandResponse, StreamEnvelope};

const MAX_CLIENT_REQUEST_BYTES: usize = super::protocol::MAX_STREAM_FRAME_BYTES;

#[derive(Clone)]
pub(crate) struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    push: ClientPushGateway,
    connection_limit: Arc<tokio::sync::Semaphore>,
}

impl ClientApiDeps {
    pub(crate) fn new(dispatch: Arc<ClientCommandDispatch>, push: ClientPushGateway) -> Self {
        Self {
            dispatch,
            push,
            connection_limit: Arc::new(tokio::sync::Semaphore::new(16)),
        }
    }
}

pub(super) fn router(deps: Option<ClientApiDeps>) -> Router {
    Router::new()
        .route(CLIENT_WS_PATH, get(upgrade))
        .with_state(deps)
}

async fn upgrade(
    State(deps): State<Option<ClientApiDeps>>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(deps) = deps else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Ok(permit) = deps.connection_limit.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let ws = super::auth::echo_bearer_subprotocol(ws, &headers)
        .max_message_size(MAX_CLIENT_REQUEST_BYTES)
        .max_frame_size(MAX_CLIENT_REQUEST_BYTES);
    let push = deps.push.subscribe();
    ws.on_upgrade(move |socket| serve(socket, deps, push, permit))
}

async fn serve(
    socket: WebSocket,
    deps: ClientApiDeps,
    mut push: ClientPushSubscription,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let (mut sink, stream) = socket.split();
    let mut responses = Box::pin(stream.then(|frame| async {
        match frame {
            Ok(Message::Text(text)) => {
                let response = dispatch_text(&deps.dispatch, &text).await;
                match serde_json::to_string(&response) {
                    Ok(response) => Some(Message::Text(response.into())),
                    Err(error) => {
                        log::error!("Client response serialization failed: {error}");
                        Some(Message::Close(None))
                    }
                }
            }
            Ok(Message::Ping(data)) => Some(Message::Pong(data)),
            Ok(Message::Pong(_)) => None,
            _ => Some(Message::Close(None)),
        }
    }));
    loop {
        let message = tokio::select! {
            response = responses.next() => match response {
                Some(Some(message)) => message,
                Some(None) => continue,
                None => break,
            },
            frame = push.recv() => match frame {
                Ok(frame) => Message::Text(frame.as_ref().to_string().into()),
                Err(error) => {
                    log::warn!("Client push subscription ended: {error}");
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            },
        };
        let closing = matches!(message, Message::Close(_));
        if sink.send(message).await.is_err() || closing {
            break;
        }
    }
}

async fn dispatch_text(dispatch: &ClientCommandDispatch, text: &str) -> CommandResponse {
    match serde_json::from_str::<ClientRequest>(text) {
        Ok(ClientRequest::Command(request)) => CommandResponse::new(
            request.request_id,
            dispatch.dispatch(&request.command, request.args).await,
        ),
        Ok(ClientRequest::Stream(StreamEnvelope::Stream { .. } | StreamEnvelope::Ack { .. })) => {
            CommandResponse::new(
                String::new(),
                Err(AppError::coded(
                    "UNSUPPORTED_STREAM",
                    "Stream transport is not enabled",
                )),
            )
        }
        Err(error) => {
            let request_id = serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .and_then(|value| value.get("request_id")?.as_str().map(str::to_string))
                .unwrap_or_default();
            CommandResponse::new(
                request_id,
                Err(AppError::coded("INVALID_REQUEST", error.to_string())),
            )
        }
    }
}
