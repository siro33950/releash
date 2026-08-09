use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tempfile::TempDir;
use tower::ServiceExt;

use super::super::test_support;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
};
use crate::domain::local_event::{
    LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent, LocalEventTransactionRepository,
    StreamId,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

fn receive_payload(armed: &ArmedProviderLifecycle) -> serde_json::Value {
    let provider = match armed.provider() {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    };
    serde_json::json!({
        "slot_id": armed.slot_id().as_str(),
        "binding_id": armed.binding_id(),
        "capability": armed.capability(),
        "provider": provider,
        "agent_session_id": armed.scope().agent_session_id(),
        "signal": {
            "event": "session_started",
            "provider_session_id": "claude-session-1",
            "transcript_ref": "provider://claude/transcript"
        }
    })
}

fn provider_lifecycle_usecase(store: &Arc<LocalEventStore>) -> Arc<ProviderLifecycleUsecase> {
    let events = Arc::new(LocalProviderLifecycleEventRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    ));
    Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        events,
    ))
}

fn slot_id() -> ProviderLifecycleSlotId {
    ProviderLifecycleSlotId::new("slot-1").unwrap()
}

async fn provider_event_count(store: &LocalEventStore, agent_session_id: &str) -> usize {
    store
        .load_stream(LoadStreamRequest {
            stream_id: StreamId::provider_lifecycle(agent_session_id).unwrap(),
            after: None,
            limit: 64,
        })
        .await
        .unwrap()
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                &event.event,
                LoadedDomainEvent::Known(inner)
                    if matches!(inner.as_ref(), LocalDomainEvent::ProviderLifecycle(_))
            )
        })
        .count()
}

#[tokio::test]
async fn test_providerライフサイクル利用不能_hook_cli未実行でも診断事実を永続化する() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let armed = usecase
        .arm(
            slot_id(),
            ProviderKind::Codex,
            ProviderLifecycleScope::new("agent-1").unwrap(),
        )
        .await
        .unwrap();
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);

    let unavailable = serde_json::json!({
        "slot_id": armed.slot_id().as_str(),
        "binding_id": armed.binding_id(),
        "capability": armed.capability(),
        "provider": "codex",
        "agent_session_id": "agent-1",
        "reason": "session_start_deadline_exceeded"
    });
    let (status, response) = test_support::send_json(
        &router,
        "/v1/provider-lifecycle/unavailable",
        unavailable.clone(),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(response, serde_json::json!({"status": "applied"}));
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);

    let (duplicate_status, duplicate_response) =
        test_support::send_json(&router, "/v1/provider-lifecycle/unavailable", unavailable).await;
    assert_eq!(duplicate_status, axum::http::StatusCode::OK);
    assert_eq!(
        duplicate_response,
        serde_json::json!({"status": "duplicate"})
    );
    assert_eq!(provider_event_count(&store, "agent-1").await, 2);
}

#[tokio::test]
async fn test_providerライフサイクルapi_認証済みrequestをusecaseへ渡す() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let armed = usecase
        .arm(
            slot_id(),
            ProviderKind::Claude,
            ProviderLifecycleScope::new("agent-1").unwrap(),
        )
        .await
        .unwrap();
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);

    let (status, response) = test_support::send_json(
        &router,
        "/v1/provider-lifecycle/signals",
        receive_payload(&armed),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(response, serde_json::json!({"status": "applied"}));
}

#[tokio::test]
async fn test_providerライフサイクルapi_未認証requestでは状態を変更しない() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let armed = usecase
        .arm(
            slot_id(),
            ProviderKind::Claude,
            ProviderLifecycleScope::new("agent-1").unwrap(),
        )
        .await
        .unwrap();
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);
    let payload = receive_payload(&armed);

    let unauthorized = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/provider-lifecycle/signals")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), axum::http::StatusCode::UNAUTHORIZED);

    let (status, response) =
        test_support::send_json(&router, "/v1/provider-lifecycle/signals", payload).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(response, serde_json::json!({"status": "applied"}));
}

#[tokio::test]
async fn test_providerライフサイクルapi_不正requestでは状態を変更しない() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let armed = usecase
        .arm(
            slot_id(),
            ProviderKind::Claude,
            ProviderLifecycleScope::new("agent-1").unwrap(),
        )
        .await
        .unwrap();
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);

    let malformed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/provider-lifecycle/signals")
                .header("authorization", "Bearer secret")
                .header("content-type", "application/json")
                .body(Body::from("{not-valid-json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(malformed.status(), axum::http::StatusCode::BAD_REQUEST);

    let (status, response) = test_support::send_json(
        &router,
        "/v1/provider-lifecycle/signals",
        receive_payload(&armed),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(response, serde_json::json!({"status": "applied"}));
}

#[tokio::test]
async fn test_providerライフサイクル解放_local_apiに外部routeを公開しない() {
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);

    let (status, response) = test_support::send_json(
        &router,
        "/v1/provider-lifecycle/slots/release",
        serde_json::json!({
            "slot_id": "slot-1",
            "binding_id": "binding-1",
        }),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        response,
        serde_json::json!({
            "code": "not_found",
            "message": "local API endpoint was not found",
        })
    );
}

#[tokio::test]
async fn test_providerライフサイクルapi_session_started受理でhook_ingress区間を記録する() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    let directory = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let usecase = provider_lifecycle_usecase(&store);
    let armed = usecase
        .arm(
            slot_id(),
            ProviderKind::Claude,
            ProviderLifecycleScope::new("agent-1").unwrap(),
        )
        .await
        .unwrap();
    let router =
        test_support::test_router_with_provider_lifecycle(directory.path(), "secret", usecase);
    crate::other::telemetry::start_terminal_launch_sample_collection();

    let (status, _) = test_support::send_json(
        &router,
        "/v1/provider-lifecycle/signals",
        receive_payload(&armed),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::OK);
    let samples = crate::other::telemetry::take_terminal_launch_samples();
    assert!(samples
        .iter()
        .any(|sample| sample.phase == "terminal.launch.hook_ingress"));
}
