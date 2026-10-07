use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, UPGRADE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use super::error::ApiError;
use crate::adaptor::presenter::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;

pub(super) async fn require_bearer(
    State(accepted): State<Arc<str>>,
    request: Request,
    next: Next,
) -> Response {
    let header_authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| bool::from(token.as_bytes().ct_eq(accepted.as_bytes())));
    // ブラウザのWebSocketはheaderを設定できないため、WS handshakeに限り
    // Sec-WebSocket-Protocol経由のbearerも受理する（terminal streamが使用）
    let subprotocol_authorized = is_websocket_handshake(&request)
        && bearer_subprotocols(request.headers()).any(|candidate| {
            bool::from(
                candidate.as_bytes()[TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX.len()..]
                    .ct_eq(accepted.as_bytes()),
            )
        });
    let authorized = header_authorized || subprotocol_authorized;
    if !authorized {
        return ApiError::unauthorized().into_response();
    }
    next.run(request).await
}

fn bearer_subprotocols(headers: &axum::http::HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| value.starts_with(TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX))
}

fn is_websocket_handshake(request: &Request) -> bool {
    request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate.eq_ignore_ascii_case("websocket"))
        })
}

#[derive(Clone)]
pub struct ClientTokens {
    pub operator: crate::infrastructure::local_api::BearerToken,
    pub hook: crate::infrastructure::local_api::BearerToken,
}
include!(concat!(env!("OUT_DIR"), "/client_scopes.rs"));

pub async fn require_client(
    State(tokens): State<ClientTokens>,
    request: Request,
    next: Next,
) -> Response {
    use axum::http::{header, Method, StatusCode};
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    let allowed = matches!(origin, Some("tauri://localhost" | "http://tauri.localhost"))
        || (cfg!(debug_assertions)
            && matches!(
                origin,
                Some("http://localhost:1420" | "http://127.0.0.1:1420")
            ));
    if request.headers().contains_key(header::ORIGIN) && !allowed {
        return (StatusCode::FORBIDDEN, "Origin is not allowed").into_response();
    }
    let origin = request.headers().get(header::ORIGIN).cloned();
    let mut response = if request.method() == Method::OPTIONS && origin.is_some() {
        let method = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_METHOD)
            .and_then(|value| value.to_str().ok());
        let headers = request
            .headers()
            .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if method != Some("POST")
            || headers
                .split(',')
                .map(str::trim)
                .filter(|header| !header.is_empty())
                .any(|header| {
                    !matches!(
                        header.to_ascii_lowercase().as_str(),
                        "authorization"
                            | "content-type"
                            | "connect-protocol-version"
                            | "connect-timeout-ms"
                            | "x-user-agent"
                            | "grpc-timeout"
                            | "x-grpc-web"
                    )
                })
        {
            return (StatusCode::FORBIDDEN, "Preflight is not allowed").into_response();
        }
        let mut response = StatusCode::NO_CONTENT.into_response();
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "POST".parse().unwrap(),
        );
        response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "authorization, content-type, connect-protocol-version, connect-timeout-ms, x-user-agent, grpc-timeout, x-grpc-web".parse().unwrap());
        response
    } else {
        let candidate = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let scope = candidate.and_then(|candidate| {
            if tokens.operator.accepts(candidate) {
                Some(crate::adaptor::presenter::client::Scope::Operator)
            } else if tokens.hook.accepts(candidate) {
                Some(crate::adaptor::presenter::client::Scope::Hook)
            } else {
                None
            }
        });
        match scope {
            None => ApiError::unauthorized().into_response(),
            Some(scope)
                if method_scopes(request.uri().path().rsplit('/').next().unwrap_or_default())
                    .is_some_and(|scopes| !scopes.contains(&scope)) =>
            {
                crate::adaptor::presenter::connect::scope_denied(request.headers())
            }
            Some(_) => next.run(request).await,
        }
    };
    if let Some(origin) = origin {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        response
            .headers_mut()
            .insert(header::VARY, "Origin".parse().unwrap());
        response.headers_mut().insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "grpc-status, grpc-message, grpc-status-details-bin"
                .parse()
                .unwrap(),
        );
    }
    response
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_tests;
