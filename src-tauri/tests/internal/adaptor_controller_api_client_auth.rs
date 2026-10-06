use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use releash_lib::test_support::integration::transport::require_client;
use tower::ServiceExt;

#[tokio::test]
pub async fn test_リクエスト認証_server停止で共有client_tokenが失効する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let binding = releash_lib::test_support::integration::transport::test_binding(
        directory.path().to_owned(),
    )
    .unwrap();
    let bearer = binding.terminal_bearer_token();
    let router = Router::new().route("/rpc", post(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(binding.client_bearer_token(), require_client),
    );
    let server = binding
        .start(router.clone(), &tokio::runtime::Handle::current())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();
    let request = || {
        Request::post("/rpc")
            .header("origin", "tauri://localhost")
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router.clone().oneshot(request()).await.unwrap().status(),
        StatusCode::OK
    );

    // When
    server.shutdown_and_wait().await.unwrap();

    // Then
    assert_eq!(
        router.oneshot(request()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}
