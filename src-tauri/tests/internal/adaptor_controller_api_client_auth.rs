use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use releashd::test_support::integration::transport::require_client;
use tower::ServiceExt;

#[tokio::test]
pub async fn test_リクエスト認証_server停止でoperatorとhook_tokenが失効する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let binding =
        releashd::test_support::integration::transport::test_binding(directory.path().to_owned())
            .unwrap();
    let bearer = binding.terminal_bearer_token();
    let hook_value = binding.test_hook_token();
    let router = Router::new().route("/rpc", post(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(
            releashd::test_support::integration::transport::ClientTokens {
                operator: binding.client_bearer_token(),
                hook: binding.hook_bearer_token(),
            },
            require_client,
        ),
    );
    let server = binding
        .start(router.clone(), &tokio::runtime::Handle::current())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();
    let request = |bearer: &str| {
        Request::post("/rpc")
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap()
    };
    for token in [bearer.as_ref(), hook_value.as_ref()] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(token))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }

    // When
    server.shutdown_and_wait().await.unwrap();

    // Then
    for token in [bearer.as_ref(), hook_value.as_ref()] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(token))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
