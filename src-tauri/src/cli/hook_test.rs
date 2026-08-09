use std::io::Cursor;
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::adaptor::controller::api::test_support as api_test_support;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderLifecycleCredentialGateway, LocalProviderLifecycleEventRepository,
    ProviderLaunchContext, ProviderLaunchSpec,
};
use crate::domain::local_event::{
    LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent, LocalEventTransactionRepository,
    StreamId,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};
use crate::infrastructure::local_api::LocalApiServerBinding;
use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

#[test]
fn test_hook受信_上限を超えるstdinを拒否する() {
    let payload = vec![b'x'; 65_537];

    let error = receive_from(Cursor::new(payload), HookProvider::Claude).unwrap_err();

    assert_eq!(
        error,
        CliError::InvalidInput(
            "Provider lifecycle payload exceeds the 65536 byte limit".to_string()
        )
    );
}

#[test]
fn test_hook受信_claude_subagent_signalはlocal_apiへ送らず成功として無視する() {
    let _lock = TEST_ENV_LOCK.lock();
    let _slot_id = EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot-1");
    let _binding_id = EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID", "binding-1");
    let _capability =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY", "capability-1");
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        "agent-session-1",
    );
    let payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/subagent-transcript",
        "cwd":"/workspace",
        "hook_event_name":"Stop",
        "agent_id":"agent-child-1",
        "agent_type":"Explore"
    }"#;

    assert_eq!(
        receive_from(Cursor::new(payload), HookProvider::Claude).unwrap(),
        "{}"
    );
}

#[test]
fn test_hook受信_local_api配送失敗をlaunch_health_markerへ記録する() {
    let _lock = TEST_ENV_LOCK.lock();
    let data = TempDir::new().unwrap();
    let marker = data
        .path()
        .join("provider-launches/agent/launch/hook-health.json");
    let _data_dir = EnvVarGuard::set_path("RELEASH_DATA_DIR", data.path());
    let _slot_id =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot-failed-delivery");
    let _binding_id = EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID", "binding-1");
    let _capability =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY", "capability-1");
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        "agent-session-1",
    );
    let _health_file =
        EnvVarGuard::set_path("RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE", marker.as_path());
    let payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"SessionStart"
    }"#;

    assert!(receive_from(Cursor::new(payload), HookProvider::Claude).is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(marker).unwrap()).unwrap(),
        serde_json::json!({
            "provider": "claude",
            "launchId": "slot-failed-delivery",
            "reason": "local_api_unavailable"
        })
    );
}

#[test]
fn test_hook受信_local_api_http失敗もlaunch_health_markerへ記録する() {
    let _lock = TEST_ENV_LOCK.lock();
    let data = TempDir::new().unwrap();
    let marker = data
        .path()
        .join("provider-launches/agent/http-failure/hook-health.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let binding = LocalApiServerBinding::bind(data.path().to_path_buf()).unwrap();
    let server = binding.start(
        axum::Router::new().route(
            "/v1/provider-lifecycle/signals",
            axum::routing::post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
        ),
        runtime.handle(),
    );
    let _data_dir = EnvVarGuard::set_path("RELEASH_DATA_DIR", data.path());
    let _slot_id =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot-http-failure");
    let _binding_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID",
        "binding-http-failure",
    );
    let _capability = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY",
        "capability-http-failure",
    );
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        "agent-session-http-failure",
    );
    let _health_file =
        EnvVarGuard::set_path("RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE", marker.as_path());
    let payload = br#"{
        "session_id":"claude-session-http-failure",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"SessionStart"
    }"#;

    assert!(receive_from(Cursor::new(payload), HookProvider::Claude).is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(marker).unwrap()).unwrap(),
        serde_json::json!({
            "provider": "claude",
            "launchId": "slot-http-failure",
            "reason": "local_api_unavailable"
        })
    );
    server.shutdown();
}

#[test]
fn test_hook受信_session_start成功だけがdelivery_failure_markerを解除する() {
    let _lock = TEST_ENV_LOCK.lock();
    let client_data = TempDir::new().unwrap();
    let store_data = TempDir::new().unwrap();
    let plugin_data = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        store_data.path().to_path_buf(),
    ))
    .unwrap();
    let events = Arc::new(LocalProviderLifecycleEventRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    ));
    let usecase = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        events,
    ));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let scope = ProviderLifecycleScope::new("agent-1").unwrap();
    let armed = runtime
        .block_on(usecase.arm(
            ProviderLifecycleSlotId::new("slot-1").unwrap(),
            ProviderKind::Claude,
            scope.clone(),
        ))
        .unwrap();
    let launch = ProviderLaunchSpec::for_provider(
        ProviderKind::Claude,
        ProviderLaunchContext::new(
            armed.slot_id().clone(),
            armed.binding_id(),
            armed.capability(),
            scope,
        )
        .unwrap(),
        "releash",
        Some(plugin_data.path()),
    )
    .unwrap();
    let binding = LocalApiServerBinding::bind(client_data.path().to_path_buf()).unwrap();
    let router = api_test_support::test_router_with_provider_lifecycle(
        store_data.path(),
        binding.bearer_token().as_ref(),
        usecase,
    );
    let server = binding.start(router, runtime.handle());
    let _data_dir = EnvVarGuard::set_path("RELEASH_DATA_DIR", client_data.path());
    let launch_value = |name: &str| {
        launch
            .environment()
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
            .unwrap()
    };
    let _slot_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_SLOT_ID",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID"),
    );
    let _binding_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_BINDING_ID"),
    );
    let _capability = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_CAPABILITY"),
    );
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID"),
    );
    let marker = client_data
        .path()
        .join("provider-launches/agent/launch/hook-health.json");
    crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
        client_data.path(),
        &marker,
        "claude",
        "slot-1",
    )
    .unwrap();
    let _health_file =
        EnvVarGuard::set_path("RELEASH_PROVIDER_LIFECYCLE_HEALTH_FILE", marker.as_path());
    let payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"SessionStart",
        "source":"startup"
    }"#;

    receive_from(Cursor::new(payload), HookProvider::Claude).unwrap();

    assert!(!marker.exists());

    crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
        client_data.path(),
        &marker,
        "claude",
        "slot-1",
    )
    .unwrap();
    let stop_payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"Stop",
        "stop_hook_active":false
    }"#;

    receive_from(Cursor::new(stop_payload), HookProvider::Claude).unwrap();

    assert!(marker.exists());

    let page = runtime
        .block_on(store.load_stream(LoadStreamRequest {
            stream_id: StreamId::provider_lifecycle("agent-1").unwrap(),
            after: None,
            limit: 64,
        }))
        .unwrap();
    let provider_events = page
        .events
        .into_iter()
        .filter(|event| {
            matches!(
                &event.event,
                LoadedDomainEvent::Known(inner)
                    if matches!(inner.as_ref(), LocalDomainEvent::ProviderLifecycle(_))
            )
        })
        .count();
    assert_eq!(provider_events, 3);

    server.shutdown();
}
