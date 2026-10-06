pub(crate) mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::super::*;

    fn protected_router() -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                Arc::<str>::from("secret"),
                require_bearer,
            ))
    }

    fn bearer_subprotocol(token: &str) -> String {
        format!("{}{token}", TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX)
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
