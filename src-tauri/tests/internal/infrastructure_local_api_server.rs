use std::error::Error;

use axum::Router;
use releash_lib::test_support::integration::transport::LocalApiServerError;

#[tokio::test]
pub async fn test_クライアントtoken_hookと分離しdiscoveryへ公開する() {
    // Given / When
    let directory = tempfile::tempdir().unwrap();
    let binding = releash_lib::test_support::integration::transport::test_binding(
        directory.path().to_path_buf(),
    )
    .unwrap();
    assert!(!binding.test_discovery_path().exists());
    let hook = binding.test_hook_token();
    let client = binding.terminal_bearer_token();
    let server = binding
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    server.publish_discovery().unwrap();
    let discovery = std::fs::read_to_string(directory.path().join("client-api.json")).unwrap();
    // Then
    assert_ne!(client, hook);
    assert!(discovery.contains(client.as_ref()));
    assert!(!discovery.contains(hook.as_ref()));
}

#[tokio::test]
pub async fn test_local_api_server起動_discovery作成失敗を専用errorで返す() {
    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("not-a-directory");
    std::fs::write(&data_path, "occupied").unwrap();

    let server = releash_lib::test_support::integration::transport::test_binding(data_path)
        .unwrap()
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    let error = server.publish_discovery().unwrap_err();

    assert!(matches!(error, LocalApiServerError::Discovery(_)));
    assert!(error.source().is_some());
}

#[tokio::test]
pub async fn test_クライアントdiscovery_公開失敗時は認証tokenを失効する() {
    let directory = tempfile::tempdir().unwrap();
    // Given
    std::fs::create_dir(directory.path().join("client-api.json")).unwrap();
    // When / Then
    let binding = releash_lib::test_support::integration::transport::test_binding(
        directory.path().to_owned(),
    )
    .unwrap();
    let client = binding.client_bearer_token();
    let hook = binding.hook_bearer_token();
    let client_token = binding.terminal_bearer_token();
    let hook_token = binding.test_hook_token();
    let server = binding
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    assert!(server.publish_discovery().is_err());
    use tower::ServiceExt;
    let tokens = releash_lib::test_support::integration::transport::ClientTokens {
        operator: client,
        hook,
    };
    let router = axum::Router::new()
        .route("/", axum::routing::post(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            tokens,
            releash_lib::test_support::integration::transport::require_client,
        ));
    for token in [client_token, hook_token] {
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::post("/")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
pub async fn test_local_api_server終了_停止を通知して所有discoveryを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let binding = releash_lib::test_support::integration::transport::test_binding(
        directory.path().to_path_buf(),
    )
    .unwrap();
    let discovery_path = binding.test_discovery_path().to_path_buf();
    let server = binding
        .start(Router::new(), &tokio::runtime::Handle::current())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();

    assert!(discovery_path.exists());
    server.shutdown_and_wait().await.unwrap();
    assert!(!discovery_path.exists());
}
