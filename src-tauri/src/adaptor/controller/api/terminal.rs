use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::adaptor::protocol::terminal::{
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum TerminalWsRequestV1 {
    GetSurface {
        id: String,
        owner: TerminalSurfaceOwnerV1,
    },
    AttachSurface {
        id: String,
        owner: TerminalSurfaceOwnerV1,
    },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum TerminalWsResponseV1 {
    Ok {
        id: String,
        surface: TerminalSurfaceV1,
    },
    Error {
        id: String,
        error: TerminalWsErrorV1,
    },
    Event {
        id: String,
        item: TerminalSurfaceStreamItemV1,
    },
}

#[derive(Serialize)]
struct TerminalWsErrorV1 {
    code: &'static str,
    message: String,
}

fn dispatch(deps: &TerminalApiDeps, request: TerminalWsRequestV1) -> TerminalWsResponseV1 {
    match request {
        TerminalWsRequestV1::GetSurface { id, owner } => match owner
            .try_into()
            .map_err(crate::usecase::terminal_surface::error::UsecaseError::Gateway)
            .and_then(|owner| deps.application.get(&owner))
        {
            Ok(surface) => TerminalWsResponseV1::Ok {
                id,
                surface: surface.into(),
            },
            Err(error) => TerminalWsResponseV1::Error {
                id,
                error: TerminalWsErrorV1 {
                    code: "PTY_ERROR",
                    message: error.to_string(),
                },
            },
        },
        TerminalWsRequestV1::AttachSurface { id, .. } => TerminalWsResponseV1::Error {
            id,
            error: TerminalWsErrorV1 {
                code: "INVALID_REQUEST",
                message: "attach_surface must be handled as a stream".to_string(),
            },
        },
    }
}

pub(super) fn router() -> Router<LocalApiState> {
    Router::new().route("/v1/terminal", get(upgrade))
}

async fn upgrade(State(state): State<LocalApiState>, ws: WebSocketUpgrade) -> Response {
    let Some(deps) = state.terminal else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Ok(permit) = deps.connection_limit.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(MAX_TERMINAL_REQUEST_BYTES + 1)
        .max_frame_size(MAX_TERMINAL_REQUEST_BYTES + 1)
        .on_upgrade(move |socket| serve(socket, deps, permit))
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
                    Ok(TerminalWsRequestV1::AttachSurface { id, owner }) => {
                        serve_attachment(&mut sink, &mut stream, &deps, id, owner).await;
                        break;
                    }
                    Ok(request) => dispatch(&deps, request),
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
) {
    let owner = match owner
        .try_into()
        .map_err(crate::usecase::terminal_surface::error::UsecaseError::Gateway)
    {
        Ok(owner) => owner,
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
    let attachment_id = format!("ws-{}", uuid::Uuid::new_v4());
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
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {}
                }
            }
        }
    }
    deps.application.detach(&attachment_id);
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
