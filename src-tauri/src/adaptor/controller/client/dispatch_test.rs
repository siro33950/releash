use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::presenter::client as wire;
use prost::Message;
use serde_json::Value;
use std::sync::Arc;

#[tokio::test]
pub async fn test_クライアントrpc_停止中はusecase実行前に拒否する() {
    use tower::ServiceExt;
    // Given
    let daemon = crate::usecase::daemon::DaemonUsecase::test_with_repository(
        crate::adaptor::gateway::daemon::serving(),
    );
    let dispatch = Arc::new(ClientCommandDispatch::new(daemon.clone()));
    let router = crate::adaptor::controller::api::client::router(
        Some(crate::test_support::client_api_deps(dispatch)),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    daemon
        .stop(crate::domain::daemon::StopRequest::Exit { code: 0 })
        .await;
    // When
    let request =
        axum::http::Request::post("/releash.client.v1.ClientService/UpdateExternalEditor")
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    // Then
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "failed_precondition");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error["details"][0]["value"].as_str().unwrap())
        .unwrap();
    let detail = wire::from_value(wire::CommandError::decode(bytes.as_slice()).unwrap()).unwrap();
    assert_eq!(detail["code"], "APPLICATION_UNAVAILABLE");
}

#[tokio::test]
pub async fn test_クライアントrpc_停止中はstreamも拒否する() {
    use tower::ServiceExt;
    // Given
    let daemon = crate::usecase::daemon::DaemonUsecase::test_with_repository(
        crate::adaptor::gateway::daemon::serving(),
    );
    let dispatch = Arc::new(ClientCommandDispatch::new(daemon.clone()));
    let router = crate::adaptor::controller::api::client::router(
        Some(crate::test_support::client_api_deps(dispatch)),
        crate::adaptor::controller::daemon::default_timeout(),
    );
    daemon
        .stop(crate::domain::daemon::StopRequest::Exit { code: 0 })
        .await;
    // When
    let request =
        axum::http::Request::post("/releash.client.v1.ClientService/StartStateSubscription")
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{}"))
            .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    // Then
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "failed_precondition");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error["details"][0]["value"].as_str().unwrap())
        .unwrap();
    let detail = wire::from_value(wire::CommandError::decode(bytes.as_slice()).unwrap()).unwrap();
    assert_eq!(detail["code"], "APPLICATION_UNAVAILABLE");
}
