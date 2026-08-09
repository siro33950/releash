use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};

use crate::adaptor::protocol::terminal::{
    TerminalSurfaceOwnerV1, TerminalWsAttachedRequestV1, TerminalWsErrorV1, TerminalWsRequestV1,
    TerminalWsResponseV1, TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX, TERMINAL_WS_PATH,
};
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

use super::LocalApiState;

const MAX_TERMINAL_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct TerminalApiDeps {
    application: Arc<TerminalSurfaceApplication>,
    connection_limit: Arc<tokio::sync::Semaphore>,
}

impl TerminalApiDeps {
    pub(crate) fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self {
            application,
            connection_limit: Arc::new(tokio::sync::Semaphore::new(16)),
        }
    }
}

pub(super) fn router() -> Router<LocalApiState> {
    Router::new().route(TERMINAL_WS_PATH, get(upgrade))
}

async fn upgrade(
    State(state): State<LocalApiState>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(deps) = state.terminal else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Ok(permit) = deps.connection_limit.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    // subprotocol認証を使うクライアントにはhandshake成立のためechoが必要
    let bearer_subprotocol = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find(|candidate| candidate.starts_with(TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX))
                .map(str::to_string)
        });
    let mut ws = ws
        .max_message_size(MAX_TERMINAL_REQUEST_BYTES + 1)
        .max_frame_size(MAX_TERMINAL_REQUEST_BYTES + 1);
    if let Some(subprotocol) = bearer_subprotocol {
        ws = ws.protocols([subprotocol]);
    }
    ws.on_upgrade(move |socket| serve(socket, deps, permit))
}

async fn serve(
    socket: WebSocket,
    deps: TerminalApiDeps,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    let (mut sink, mut stream) = socket.split();
    while let Some(frame) = stream.next().await {
        let response = match frame {
            Ok(Message::Text(text)) if text.len() <= MAX_TERMINAL_REQUEST_BYTES => {
                match serde_json::from_str::<TerminalWsRequestV1>(&text) {
                    Ok(TerminalWsRequestV1::AttachSurface {
                        id,
                        owner,
                        attachment_id,
                    }) => {
                        serve_attachment(&mut sink, &mut stream, &deps, id, owner, attachment_id)
                            .await;
                        break;
                    }
                    Err(error) => invalid_request(error.to_string()),
                }
            }
            Ok(Message::Text(_)) | Ok(Message::Binary(_)) => {
                invalid_request("Terminal WebSocket accepts bounded JSON text requests".to_string())
            }
            Ok(Message::Ping(payload)) => {
                if sink.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
                continue;
            }
            Ok(Message::Pong(_)) => continue,
            Ok(Message::Close(_)) | Err(_) => break,
        };
        if send_response(&mut sink, &response).await.is_err() {
            break;
        }
    }
}

async fn send_response(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    response: &TerminalWsResponseV1,
) -> Result<(), ()> {
    let payload = serde_json::to_string(response).map_err(|_| ())?;
    sink.send(Message::Text(payload.into()))
        .await
        .map_err(|_| ())
}

async fn serve_attachment(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    deps: &TerminalApiDeps,
    id: String,
    owner: TerminalSurfaceOwnerV1,
    attachment_id: Option<String>,
) {
    let owner = match owner.try_into() {
        Ok(owner) => owner,
        Err(message) => {
            let _ = send_response(
                sink,
                &TerminalWsResponseV1::Error {
                    id,
                    error: TerminalWsErrorV1 {
                        code: "INVALID_REQUEST",
                        message,
                    },
                },
            )
            .await;
            return;
        }
    };
    let attachment_id = attachment_id.unwrap_or_else(|| format!("ws-{}", uuid::Uuid::new_v4()));
    let mut attachment = match deps.application.attach(&attachment_id, &owner) {
        Ok(attachment) => attachment,
        Err(error) => {
            let _ = send_response(
                sink,
                &TerminalWsResponseV1::Error {
                    id,
                    error: TerminalWsErrorV1 {
                        code: "PTY_ERROR",
                        message: error.to_string(),
                    },
                },
            )
            .await;
            return;
        }
    };
    if send_response(sink, &TerminalWsResponseV1::Attached { id: id.clone() })
        .await
        .is_err()
    {
        deps.application.detach(&attachment_id);
        return;
    }

    loop {
        tokio::select! {
            item = attachment.next() => {
                let Some(item) = item else {
                    break;
                };
                if send_response(
                    sink,
                    &TerminalWsResponseV1::Event {
                        id: id.clone(),
                        item: item.into(),
                    },
                )
                .await
                .is_err()
                {
                    break;
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Ping(payload))) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(Message::Text(text)))
                        if text.len() <= MAX_TERMINAL_REQUEST_BYTES =>
                    {
                        if let Some(error_response) = handle_attached_request(deps, &text) {
                            if send_response(sink, &error_response).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {}
                }
            }
        }
    }
    deps.application.detach(&attachment_id);
}

/// attach確立後のwrite/ack/resizeを処理する。成功時は応答なし
/// （hot pathを往復させない）。失敗時のみErrorを返す。
fn handle_attached_request(deps: &TerminalApiDeps, text: &str) -> Option<TerminalWsResponseV1> {
    let request = match serde_json::from_str::<TerminalWsAttachedRequestV1>(text) {
        Ok(request) => request,
        Err(error) => return Some(invalid_request(error.to_string())),
    };
    let error = |operation: &'static str, message: String| {
        Some(TerminalWsResponseV1::Error {
            id: operation.to_string(),
            error: TerminalWsErrorV1 {
                code: "PTY_ERROR",
                message,
            },
        })
    };
    let invalid_owner = |operation: &'static str, message: String| {
        Some(TerminalWsResponseV1::Error {
            id: operation.to_string(),
            error: TerminalWsErrorV1 {
                code: "INVALID_REQUEST",
                message,
            },
        })
    };
    match request {
        TerminalWsAttachedRequestV1::Write {
            owner,
            attachment_id,
            sequence,
            data,
            client_started_at_unix_ms,
        } => {
            let owner = match owner.try_into() {
                Ok(owner) => owner,
                Err(message) => return invalid_owner("write", message),
            };
            match deps.application.write_attached(
                &owner,
                &attachment_id,
                sequence,
                client_started_at_unix_ms,
                &data,
            ) {
                Ok(()) => None,
                Err(cause) => error("write", cause.to_string()),
            }
        }
        TerminalWsAttachedRequestV1::Ack {
            attachment_id,
            sequence,
        } => {
            deps.application
                .acknowledge_output(&attachment_id, sequence);
            None
        }
        TerminalWsAttachedRequestV1::Resize { owner, rows, cols } => {
            let owner = match owner.try_into() {
                Ok(owner) => owner,
                Err(message) => return invalid_owner("resize", message),
            };
            match deps.application.resize(&owner, rows, cols) {
                Ok(()) => None,
                Err(cause) => error("resize", cause.to_string()),
            }
        }
    }
}

fn invalid_request(message: String) -> TerminalWsResponseV1 {
    TerminalWsResponseV1::Error {
        id: "invalid".to_string(),
        error: TerminalWsErrorV1 {
            code: "INVALID_REQUEST",
            message,
        },
    }
}

#[cfg(test)]
#[path = "terminal_test.rs"]
mod terminal_tests;
