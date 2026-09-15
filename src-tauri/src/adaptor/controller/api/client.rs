use super::client_operation::{decision, input as operation_input, response as operation_response};
use crate::domain::client_operation::policy;
use crate::domain::client_operation::registry::RecoveryAttempt;
use crate::usecase::client_operation::{ClientOperationUsecase, OperationCompletion};
use std::sync::Arc;

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

use crate::adaptor::controller::api::client_stream::{
    invalid, TerminalApiDeps, TerminalConnection,
};

const MAX_CLIENT_REQUEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ClientApiDeps {
    dispatch: Arc<ClientCommandDispatch>,
    operations: Arc<ClientOperationUsecase<wire::Envelope>>,
    push: ClientPushGateway,
    terminal: Option<TerminalApiDeps>,
    connection_limit: Arc<tokio::sync::Semaphore>,
    request_limit: Arc<tokio::sync::Semaphore>,
}

impl ClientApiDeps {
    pub(crate) fn new(dispatch: Arc<ClientCommandDispatch>, push: ClientPushGateway) -> Self {
        let stop_dispatch = dispatch.clone();
        let operations = ClientOperationUsecase::new(
            uuid::Uuid::new_v4().to_string(),
            Arc::new(super::client_operation::now_ms),
            Arc::new(move |id| {
                let dispatch = stop_dispatch.clone();
                Box::pin(async move {
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
                })
            }),
        );
        Self {
            dispatch,
            operations: Arc::new(operations),
            push,
            terminal: None,
            connection_limit: Arc::new(tokio::sync::Semaphore::new(16)),
            request_limit: Arc::new(tokio::sync::Semaphore::new(64)),
        }
    }

    pub(super) fn with_terminal(mut self, terminal: Option<TerminalApiDeps>) -> Self {
        self.terminal = terminal;
        self
    }
}

pub(crate) fn router(deps: Option<ClientApiDeps>) -> Router {
    if let Some(deps) = &deps {
        tokio::spawn(super::client_operation::maintain(Arc::downgrade(
            &deps.operations,
        )));
    }
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
    let mut terminal = TerminalConnection::new(deps.terminal);
    let mut requests = FuturesUnordered::new();
    let connection_id = uuid::Uuid::new_v4().to_string();
    let mut push_open = true;
    let hello = Envelope {
        body: Some(Body::Hello(wire::ClientHello {
            instance_id: deps.operations.instance_id.to_string(),
            heartbeat_interval_ms: policy::HEARTBEAT_INTERVAL_MS,
            heartbeat_timeout_ms: policy::HEARTBEAT_TIMEOUT_MS,
            connect_timeout_ms: policy::CONNECT_TIMEOUT_MS,
            reconnect_interval_ms: policy::RECONNECT_INTERVAL_MS,
            sleep_gap_ms: policy::SLEEP_GAP_MS,
            tick_interval_ms: policy::TICK_INTERVAL_MS,
            policies: deps
                .dispatch
                .command_names()
                .map(|name| {
                    (
                        name.into(),
                        wire::CommandPolicy {
                            waits_for_result: policy::waits_for_result(name),
                            polls_result: policy::polls_result(name),
                            disconnect: policy::disconnect_action(name, false).into(),
                            expired_disconnect: policy::disconnect_action(name, true).into(),
                        },
                    )
                })
                .collect(),
            deadlines_ms: deps
                .dispatch
                .command_names()
                .map(|name| (name.into(), policy::deadline_ms(name)))
                .collect(),
        })),
    };

    loop {
        deps.operations.maintain().await;
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
                    Ok(Some(Body::Hello(_))) => hello.clone(),
                    Ok(Some(Body::Heartbeat(heartbeat))) => Envelope { body: Some(Body::Heartbeat(heartbeat)) },
                    Ok(Some(Body::OperationQuery(query))) => {
                        let mut response = match &query.request {
                        Some(request) => match operation_input(request) {
                            Ok((identity, predecessors)) => match super::client_operation::references(&request.successors) {
                                Ok(successors) => decision(&query.request_id,
                                    deps.operations.prepare(&query.request_id, &query.instance_id, &identity, &predecessors, &RecoveryAttempt {
                                        connection: &connection_id, sent: query.sent,
                                        expired: query.expired || (request.deadline_unix_ms > 0 && super::client_operation::now_ms() >= request.deadline_unix_ms),
                                        user_retry: request.user_retry, successors: &successors,
                                    }), &identity),
                                Err(error) => wire::response(query.request_id, Err(error)),
                            },
                            Err(error) => wire::response(query.request_id, Err(error)),
                        },
                        None => operation_response(&query.request_id, deps.operations.query(&query.request_id, &query.instance_id)),
                        };
                        if let Some(Body::OperationStatus(status)) = &mut response.body {
                            status.query_id = query.query_id;
                        }
                        response
                    },
                    Ok(Some(Body::Request(request))) => {
                        let id = request.request_id.clone();
                        let (identity, predecessors) = match operation_input(&request) {
                            Ok(input) => input,
                            Err(error) => {
                                if sink.send(Message::Binary(wire::response(id, Err(error)).encode_to_vec().into())).await.is_err() { break; }
                                continue;
                            }
                        };
                        let successors = match super::client_operation::references(&request.successors) {
                            Ok(items) => items,
                            Err(error) => {
                                if sink.send(Message::Binary(wire::response(id, Err(error)).encode_to_vec().into())).await.is_err() { break; }
                                continue;
                            }
                        };
                        let command = request.command.expect("validated command");
                        let terminal_request = matches!(&command, wire::command_request::Command::AttachTerminalSurface(_) | wire::command_request::Command::DetachTerminalSurface(_));
                        let stopped_watch = match &command { wire::command_request::Command::StopWatching(args) => args.watcher_id, _ => None };
                        let result_id = id.clone();
                        let execution = deps.operations.execute(id.clone(), &request.instance_id, identity, &predecessors, &RecoveryAttempt {
                            connection: &connection_id, sent: request.recover,
                            expired: request.deadline_unix_ms > 0 && super::client_operation::now_ms() >= request.deadline_unix_ms,
                            user_retry: request.user_retry, successors: &successors,
                        }, || {
                            let result = if let Err(error) = deps.dispatch.admit(command.name()) {
                                futures_util::future::Either::Left(std::future::ready(Err(error)))
                            } else if matches!(&command, wire::command_request::Command::AttachTerminalSurface(_) | wire::command_request::Command::DetachTerminalSurface(_)) {
                                futures_util::future::Either::Left(std::future::ready(terminal.dispatch(command)))
                            } else {
                                let permit = deps.request_limit.clone().try_acquire_owned();
                                let dispatch = permit.as_ref().ok().map(|_| deps.dispatch.dispatch(command));
                                futures_util::future::Either::Right(async move {
                                    let _permit = permit.map_err(|_| wire::CommandError::from(crate::other::AppError::coded("REQUEST_LIMIT", "Too many pending client commands")))?;
                                    dispatch.expect("request permit").await
                                })
                            };
                            async move {
                                let result = result.await;
                                let started_watch = match &result {
                                    Ok(wire::command_result::Command::StartWatching(value) | wire::command_result::Command::StartGitDirWatching(value)) => value.value,
                                    _ => None,
                                };
                                let stopped_watch = result.is_ok().then_some(stopped_watch).flatten();
                                OperationCompletion { result: wire::response(result_id, result), started_watch, stopped_watch }
                            }
                        });
                        if terminal_request {
                            operation_response(&id, execution.await)
                        } else {
                            requests.push(tokio::spawn(async move { operation_response(&id, execution.await) }));
                            continue;
                        }
                    }
                    Ok(Some(Body::RequestAck(ack))) => {
                        let active = deps.operations.acknowledge(&ack.request_id, ack.release_watch).await;
                        if !ack.confirm_watch { continue; }
                        Envelope { body: Some(Body::OperationStatus(wire::OperationStatus {
                            request_id: ack.request_id,
                            state: if active { "watch_active" } else { "watch_released" }.into(),
                            ..Default::default()
                        })) }
                    }
                    Ok(Some(Body::Ack(ack))) => {
                        match terminal.acknowledge(&ack.attachment_id, ack.sequence) {
                            Ok(()) => {
                                if let Some(sequence) = ack.output_sequence {
                                    terminal.acknowledge_output(&ack.attachment_id, sequence);
                                }
                                continue;
                            },
                            Err(error) => wire::response(String::new(), Err(error)),
                        }
                    }
                    _ => wire::response(String::new(), Err(invalid("Invalid client envelope"))),
                }
            }
            response = requests.next(), if !requests.is_empty() => {
                match response.expect("pending command") {
                    Ok(response) => response,
                    Err(error) => { log::error!("Client command task failed: {error}"); break; }
                }
            }
            event = terminal.receiver.recv() => {
                if let Some(frame) = event.and_then(|frame| terminal.receive(frame)) {
                    if sink.send(Message::Binary(frame.into())).await.is_err() { break; }
                }
                continue;
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
    deps.operations.disconnect(&connection_id).await;
    let _ = sink.send(Message::Close(None)).await;
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;
