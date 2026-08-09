use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, UPGRADE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use super::error::ApiError;

#[derive(Clone)]
pub(super) struct AcceptedBearerTokens(Arc<[Arc<str>]>);

impl AcceptedBearerTokens {
    pub(super) fn new(tokens: impl IntoIterator<Item = Arc<str>>) -> Self {
        Self(tokens.into_iter().collect())
    }

    fn accepts(&self, candidate: &str) -> bool {
        self.0
            .iter()
            .any(|token| bool::from(candidate.as_bytes().ct_eq(token.as_ref().as_bytes())))
    }
}

pub(super) async fn require_bearer(
    State(accepted): State<AcceptedBearerTokens>,
    request: Request,
    next: Next,
) -> Response {
    let header_authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| accepted.accepts(token));
    // ブラウザのWebSocketはheaderを設定できないため、WS handshakeに限り
    // Sec-WebSocket-Protocol経由のbearerも受理する（terminal streamが使用）
    let subprotocol_authorized = is_websocket_handshake(&request)
        && request
            .headers()
            .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(',').map(str::trim).any(|candidate| {
                    candidate
                        .strip_prefix(
                            crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX,
                        )
                        .is_some_and(|token| accepted.accepts(token))
                })
            });
    let authorized = header_authorized || subprotocol_authorized;
    if !authorized {
        return ApiError::unauthorized().into_response();
    }
    next.run(request).await
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::*;

    fn protected_router() -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                AcceptedBearerTokens::new([Arc::<str>::from("secret")]),
                require_bearer,
            ))
    }

    fn bearer_subprotocol(token: &str) -> String {
        format!(
            "{}{token}",
            crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX
        )
    }

    #[tokio::test]
    async fn b073_bearer_middleware_rejects_missing_and_wrong_tokens() {
        for authorization in [None, Some("Bearer wrong")] {
            let mut request = Request::builder().uri("/");
            if let Some(value) = authorization {
                request = request.header("authorization", value);
            }
            let response = protected_router()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn b073_bearer_middleware_accepts_the_discovery_token() {
        let response = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_terminal_ws_subprotocol_bearerを受理し不一致tokenは拒否する() {
        let accepted = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("connection", "Upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-protocol", bearer_subprotocol("secret"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);

        let rejected = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("connection", "Upgrade")
                    .header("upgrade", "websocket")
                    .header("sec-websocket-protocol", bearer_subprotocol("wrong"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_subprotocol_bearerはws_handshake以外の通常httpでは認証されない() {
        let response = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("sec-websocket-protocol", bearer_subprotocol("secret"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_upgradeヘッダのwebsocket判定は大文字小文字と複数値を許容する() {
        let response = protected_router()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("connection", "Upgrade")
                    .header("upgrade", "h2c, WebSocket")
                    .header("sec-websocket-protocol", bearer_subprotocol("secret"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
