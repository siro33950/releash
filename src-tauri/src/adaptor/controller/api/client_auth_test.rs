use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use tower::ServiceExt;

fn fixture() -> (Router, crate::infrastructure::local_api::ClientBearerToken) {
    let token =
        crate::infrastructure::local_api::ClientBearerToken::from(Arc::<str>::from("client"));
    let router = Router::new().route("/rpc", post(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(token.clone(), require_client),
    );
    (router, token)
}

#[tokio::test]
async fn test_リクエスト認証_server停止で共有client_tokenが失効する() {
    use crate::infrastructure::local_api::LocalApiServerBinding;

    // Given
    let directory = tempfile::tempdir().unwrap();
    let binding = LocalApiServerBinding::bind(directory.path().to_owned()).unwrap();
    let bearer = binding.terminal_bearer_token();
    let router = Router::new().route("/rpc", post(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(binding.client_bearer_token(), require_client),
    );
    let server = binding.start(router.clone(), &tokio::runtime::Handle::current());
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

#[tokio::test]
async fn test_リクエスト認証_許可originとtokenを毎回検証し失効後の次の要求を拒否する() {
    // Given
    let (router, token) = fixture();
    for origin in ["tauri://localhost", "http://tauri.localhost"] {
        // When
        let response = router
            .clone()
            .oneshot(
                Request::post("/rpc")
                    .header("origin", origin)
                    .header("authorization", "Bearer client")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Then
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["access-control-allow-origin"], origin);
        assert!(response.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .split(", ")
            .any(|header| header == "releash-desktop-settings-changed"));
    }
    for (origin, bearer, expected) in [
        (
            Some("https://attacker.example"),
            "client",
            StatusCode::FORBIDDEN,
        ),
        (None, "client", StatusCode::FORBIDDEN),
        (
            Some("tauri://localhost"),
            "master",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let mut request = Request::post("/rpc").header("authorization", format!("Bearer {bearer}"));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        assert_eq!(
            router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    // When
    token.revoke();
    let response = router
        .oneshot(
            Request::post("/rpc")
                .header("origin", "tauri://localhost")
                .header("authorization", "Bearer client")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_preflight_許可originのpostだけtokenなしで受理し業務処理は実行しない() {
    // Given
    let (router, _) = fixture();
    for (origin, method, headers, expected) in [
        (
            "tauri://localhost",
            "POST",
            "Authorization, Content-Type, Connect-Protocol-Version",
            StatusCode::NO_CONTENT,
        ),
        (
            "http://tauri.localhost",
            "POST",
            "content-type",
            StatusCode::NO_CONTENT,
        ),
        (
            "https://attacker.example",
            "POST",
            "content-type",
            StatusCode::FORBIDDEN,
        ),
        (
            "tauri://localhost",
            "DELETE",
            "content-type",
            StatusCode::FORBIDDEN,
        ),
        (
            "tauri://localhost",
            "POST",
            "x-untrusted",
            StatusCode::FORBIDDEN,
        ),
    ] {
        // When
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/rpc")
                    .header("origin", origin)
                    .header("access-control-request-method", method)
                    .header("access-control-request-headers", headers)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Then
        assert_eq!(response.status(), expected);
    }
}
