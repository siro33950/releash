use std::{collections::HashMap, sync::Arc};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Router};
use futures_util::{stream::FuturesUnordered, SinkExt, StreamExt};
use prost::Message as _;

use crate::adaptor::controller::api::protocol::client::{self as wire, envelope::Body, Envelope};
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::gateway::push::{ClientPushError, ClientPushGateway, ClientPushSubscription};
use crate::adaptor::protocol::client::CLIENT_WS_PATH;

use crate::adaptor::controller::client::invalid_request as invalid;

const MAX_CLIENT_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    push: ClientPushGateway,
    connection_limit: Arc<tokio::sync::Semaphore>,
    request_limit: Arc<tokio::sync::Semaphore>,
    // ponytail: saves share one queue across connections; split by worktree if they block each other.
    save_tail: Arc<parking_lot::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>>,
}

impl ClientApiDeps {
    pub(crate) fn new(dispatch: Arc<ClientCommandDispatch>, push: ClientPushGateway) -> Self {
        Self {
            dispatch,
            push,
            connection_limit: Arc::new(tokio::sync::Semaphore::new(16)),
            request_limit: Arc::new(tokio::sync::Semaphore::new(64)),
            save_tail: Arc::new(parking_lot::Mutex::new(None)),
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
    let (mut sink, mut input) = socket.split();
    let mut requests = FuturesUnordered::new();
    let mut unreceived_watches: HashMap<String, UnreceivedWatch> = HashMap::new();
    let mut push_open = true;
    loop {
        let envelope = tokio::select! {
            frame = input.next() => {
                let data = match frame {
                    Some(Ok(Message::Binary(data))) => data,
                    Some(Ok(Message::Ping(data))) => {
                        if sink.send(Message::Pong(data)).await.is_err() { break; }
                        continue;
                    }
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => break,
                };
                match Envelope::decode(data).map(|envelope| envelope.body) {
                    Ok(Some(Body::Request(request))) => {
                        let id = request.request_id.clone();
                        match request.command {
                            Some(command) => {
                                if let Err(error) = deps.dispatch.admit(command.name()) {
                                    wire::response(id, Err(error))
                                } else if let Ok(permit) = deps.request_limit.clone().try_acquire_owned() {
                                    let dispatch = deps.dispatch.clone();
                                    let (previous_save, save_done) = if matches!(&command, wire::command_request::Command::SaveWorkspaceState(_)) {
                                        let (done, next) = tokio::sync::oneshot::channel::<()>();
                                        (deps.save_tail.lock().replace(next), Some(done))
                                    } else { (None, None) };
                                    requests.push(tokio::spawn(async move {
                                        if let Some(previous) = previous_save { let _ = previous.await; }
                                        let result = dispatch.dispatch(command).await;
                                        drop(save_done);
                                        let watcher_id = match &result {
                                            Ok(wire::command_result::Command::StartWatching(value) | wire::command_result::Command::StartGitDirWatching(value)) => value.value,
                                            _ => None,
                                        };
                                        let watch = watcher_id.map(|id| UnreceivedWatch { id: Some(id), dispatch, _permit: permit });
                                        (wire::response(id, result), watch)
                                    }));
                                    continue;
                                } else {
                                    wire::response(id, Err(crate::other::AppError::coded("REQUEST_LIMIT", "Too many pending client commands").into()))
                                }
                            }
                            None => wire::response(id, Err(invalid("Missing command"))),
                        }
                    }
                    Ok(Some(Body::RequestAck(ack))) => {
                        if let Some(mut watch) = unreceived_watches.remove(&ack.request_id) {
                            watch.id = None;
                        }
                        continue;
                    }
                    _ => wire::response(String::new(), Err(invalid("Invalid client envelope"))),
                }
            }
            response = requests.next(), if !requests.is_empty() => {
                match response.expect("pending command") {
                    Ok((response, watch)) => {
                        if let (Some(watch), Some(Body::Response(result))) = (watch, &response.body) {
                            unreceived_watches.insert(result.request_id.clone(), watch);
                        }
                        response
                    },
                    Err(error) => { log::error!("Client command task failed: {error}"); break; }
                }
            }
            frame = push.recv(), if push_open => match frame {
                Ok(frame) => {
                    if sink.send(Message::Binary(frame.as_ref().to_vec().into())).await.is_err() { break; }
                    continue;
                },
                Err(ClientPushError::Lagged(count)) => {
                    log::warn!("Client missed {count} pushes; refreshing current state");
                    push.resubscribe();
                    Envelope { body: Some(Body::PushResync(wire::Unit {})) }
                }
                Err(ClientPushError::Closed) => { push_open = false; continue; }
            },
        };
        if sink
            .send(Message::Binary(envelope.encode_to_vec().into()))
            .await
            .is_err()
        {
            break;
        }
    }
    let _ = sink.send(Message::Close(None)).await;
}

struct UnreceivedWatch {
    id: Option<u64>,
    dispatch: Arc<ClientCommandDispatch>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for UnreceivedWatch {
    fn drop(&mut self) {
        let Some(id) = self.id else {
            return;
        };
        let dispatch = self.dispatch.clone();
        tokio::spawn(async move {
            if let Err(error) = dispatch
                .dispatch_admitted(wire::command_request::Command::StopWatching(
                    wire::StopWatchingRequest {
                        watcher_id: Some(id),
                    },
                ))
                .await
            {
                log::error!("Unreceived watcher {id} cleanup failed: {error:?}");
            }
        });
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;
