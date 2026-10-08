mod restored_memory_tests {
    use super::super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    fn fixture() -> (Router, crate::infrastructure::local_api::BearerToken) {
        let token = crate::infrastructure::local_api::BearerToken::from(Arc::<str>::from("client"));
        let router = Router::new().route("/rpc", post(|| async { "ok" })).layer(
            axum::middleware::from_fn_with_state(
                ClientTokens {
                    operator: token.clone(),
                    hook: Arc::<str>::from("hook").into(),
                },
                require_client,
            ),
        );
        (router, token)
    }

    #[tokio::test]
    pub async fn test_リクエスト認証_許可originとtokenを毎回検証し失効後の次の要求を拒否する() {
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
                .any(|header| header == "grpc-status-details-bin"));
        }
        for (origin, bearer, expected) in [
            (
                Some("https://attacker.example"),
                "client",
                StatusCode::FORBIDDEN,
            ),
            (None, "client", StatusCode::OK),
            (
                Some("tauri://localhost"),
                "master",
                StatusCode::UNAUTHORIZED,
            ),
        ] {
            let mut request =
                Request::post("/rpc").header("authorization", format!("Bearer {bearer}"));
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            if origin.is_none() {
                assert!(!response
                    .headers()
                    .contains_key("access-control-allow-origin"));
            }
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
    pub async fn test_preflight_許可originのpostだけtokenなしで受理し業務処理は実行しない() {
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
}

#[tokio::test]
async fn test_scope認証_全methodの権限と未知methodとhook失効を区別する() {
    use super::{require_client, ClientTokens};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::Arc;
    use tower::ServiceExt;
    // Given
    let operator =
        crate::infrastructure::local_api::BearerToken::from(Arc::<str>::from("operator"));
    let hook = crate::infrastructure::local_api::BearerToken::from(Arc::<str>::from("hook"));
    let router = Router::new()
        .route(
            "/releash.client.v1.ClientService/{method}",
            post(|| async { StatusCode::NO_CONTENT }),
        )
        .layer(axum::middleware::from_fn_with_state(
            ClientTokens {
                operator,
                hook: hook.clone(),
            },
            require_client,
        ));
    // When / Then
    for (method, token, status) in [
        ("GetServerInfo", "hook", StatusCode::NO_CONTENT),
        ("InstallCli", "operator", StatusCode::NO_CONTENT),
        ("CheckLoginRegistration", "operator", StatusCode::NO_CONTENT),
        ("InstallCli", "hook", StatusCode::FORBIDDEN),
        ("CheckLoginRegistration", "hook", StatusCode::FORBIDDEN),
        ("ReceiveProviderSignal", "hook", StatusCode::NO_CONTENT),
        ("StartStateSubscription", "hook", StatusCode::FORBIDDEN),
        ("OpenStateStream", "hook", StatusCode::FORBIDDEN),
        ("ReceiveProviderSignal", "operator", StatusCode::FORBIDDEN),
        ("WorkflowGetOutput", "hook", StatusCode::NO_CONTENT),
        ("FutureMethod", "operator", StatusCode::NO_CONTENT),
        ("GetServerInfo", "wrong", StatusCode::UNAUTHORIZED),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::post(format!("/releash.client.v1.ClientService/{method}"))
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/proto")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{method} {token}");
        if status == StatusCode::FORBIDDEN {
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["code"],
                "permission_denied"
            );
        }
    }
    // When
    hook.revoke();
    let response = router
        .oneshot(
            Request::post("/releash.client.v1.ClientService/GetServerInfo")
                .header("authorization", "Bearer hook")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Then
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[path = "../../../../build.rs"]
mod build_script;

#[test]
fn test_rpcのscope定義_空と未指定と重複を拒否し有効な集合を受け付ける() {
    use crate::adaptor::presenter::client::Scope;
    // Given
    let unspecified = Scope::Unspecified as i32;
    let operator = Scope::Operator as i32;
    let hook = Scope::Hook as i32;
    // When / Then
    for (scopes, valid) in [
        (vec![], false),
        (vec![unspecified], false),
        (vec![operator, unspecified], false),
        (vec![operator, operator], false),
        (vec![operator], true),
        (vec![hook], true),
        (vec![operator, hook], true),
        (vec![hook, operator], true),
        (vec![i32::MAX], false),
    ] {
        assert_eq!(
            build_script::valid_method_scopes(&scopes, unspecified, |value| Scope::try_from(value)
                .is_ok()),
            valid,
            "{scopes:?}"
        );
    }
}
