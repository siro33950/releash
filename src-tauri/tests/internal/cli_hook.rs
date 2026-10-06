use std::io::Cursor;
use std::sync::Arc;

use tempfile::TempDir;

use releash_lib::test_support::integration::platform::receive_from;
use releash_lib::test_support::integration::platform::HookProvider;
use releash_lib::test_support::integration::platform::ProviderLifecycleReceiveResponse;
use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::providers::ProviderLifecycleScope;
use releash_lib::test_support::integration::transport::ProviderActivityRequest;
use releash_lib::test_support::integration::transport::ProviderLifecycleProvider;
use releash_lib::test_support::integration::transport::ProviderLifecycleReceiveRequest;
use releash_lib::test_support::integration::transport::ProviderLifecycleSignalRequest;

use crate::adaptor_controller_api_mod::test_support as api_test_support;
use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway;
use releash_lib::test_support::integration::providers::LocalProviderLifecycleEventRepository;
use releash_lib::test_support::integration::providers::ProviderLaunchContext;
use releash_lib::test_support::integration::providers::ProviderLaunchSpec;

use releash_lib::test_support::integration::platform::LoadStreamRequest;
use releash_lib::test_support::integration::platform::LoadedDomainEvent;
use releash_lib::test_support::integration::platform::LocalDomainEvent;
use releash_lib::test_support::integration::platform::StreamId;
use releash_lib::test_support::integration::repository::LocalEventTransactionRepository;

use releash_lib::test_support::integration::platform::EnvVarGuard;
use releash_lib::test_support::integration::platform::TEST_ENV_LOCK;
use releash_lib::test_support::integration::providers::ProviderLifecycleSlotId;
use releash_lib::test_support::integration::providers::ProviderLifecycleUsecase;

#[test]
pub fn test_hook受信_local_api配送失敗をlaunch_health_markerへ記録する() {
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
pub fn test_hook受信_local_api_http失敗もlaunch_health_markerへ記録する() {
    let _lock = TEST_ENV_LOCK.lock();
    let data = TempDir::new().unwrap();
    let marker = data
        .path()
        .join("provider-launches/agent/http-failure/hook-health.json");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let binding =
        releash_lib::test_support::integration::transport::test_binding(data.path().to_path_buf())
            .unwrap();
    let server = binding
        .start(
            axum::Router::new().route(
                "/v1/provider-lifecycle/signals",
                axum::routing::post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
            ),
            runtime.handle(),
        )
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();
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
pub fn test_hook受信_session_start成功だけがdelivery_failure_markerを解除する() {
    let _lock = TEST_ENV_LOCK.lock();
    let client_data = TempDir::new().unwrap();
    let store_data = TempDir::new().unwrap();
    let plugin_data = TempDir::new().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        store_data.path().to_path_buf(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let events = Arc::new(LocalProviderLifecycleEventRepository::new(
        releash_lib::test_support::integration::platform::shared().clone(),
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
    let binding = releash_lib::test_support::integration::transport::test_binding(
        client_data.path().to_path_buf(),
    )
    .unwrap();
    let router = api_test_support::test_router_with_provider_lifecycle(
        store_data.path(),
        binding.bearer_token().as_ref(),
        usecase,
    );
    let server = binding
        .start(router, runtime.handle())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();
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
    releash_lib::test_support::integration::providers::write_local_api_failure(
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

    releash_lib::test_support::integration::providers::write_local_api_failure(
        client_data.path(),
        &marker,
        "claude",
        "slot-1",
    )
    .unwrap();
    let activity_payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"PermissionRequest",
        "tool_name":"Bash"
    }"#;

    assert_eq!(
        receive_from(Cursor::new(activity_payload), HookProvider::Claude).unwrap(),
        "{}"
    );
    assert!(marker.exists());

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

#[test]
pub fn test_hook受信_両providerの活動eventを期待するactivityとしてlocal_apiへ送る() {
    let _lock = TEST_ENV_LOCK.lock();
    let data = TempDir::new().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let binding =
        releash_lib::test_support::integration::transport::test_binding(data.path().to_path_buf())
            .unwrap();
    let requests = Arc::new(std::sync::Mutex::new(
        Vec::<ProviderLifecycleReceiveRequest>::new(),
    ));
    let route_requests = requests.clone();
    let router = axum::Router::new().route(
        "/v1/provider-lifecycle/signals",
        axum::routing::post(
            move |axum::Json(request): axum::Json<ProviderLifecycleReceiveRequest>| {
                let route_requests = route_requests.clone();
                async move {
                    route_requests.lock().unwrap().push(request);
                    axum::Json(ProviderLifecycleReceiveResponse::Applied)
                }
            },
        ),
    );
    let server = binding
        .start(router, runtime.handle())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();
    let _data_dir = EnvVarGuard::set_path("RELEASH_DATA_DIR", data.path());
    let _slot_id =
        EnvVarGuard::set_value("RELEASH_PROVIDER_LIFECYCLE_SLOT_ID", "slot-awaiting-answer");
    let _binding_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_BINDING_ID",
        "binding-awaiting-answer",
    );
    let _capability = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_CAPABILITY",
        "capability-awaiting-answer",
    );
    let _agent_session_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_AGENT_SESSION_ID",
        "agent-session-awaiting-answer",
    );
    let cases = [
        (
            HookProvider::Claude,
            ProviderLifecycleProvider::Claude,
            ProviderActivityRequest::AwaitingAnswer,
            serde_json::json!({
                "session_id": "claude-session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash"
            }),
        ),
        (
            HookProvider::Claude,
            ProviderLifecycleProvider::Claude,
            ProviderActivityRequest::AwaitingAnswer,
            serde_json::json!({
                "session_id": "claude-session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "AskUserQuestion"
            }),
        ),
        (
            HookProvider::Codex,
            ProviderLifecycleProvider::Codex,
            ProviderActivityRequest::AwaitingAnswer,
            serde_json::json!({
                "session_id": "codex-session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash"
            }),
        ),
        (
            HookProvider::Codex,
            ProviderLifecycleProvider::Codex,
            ProviderActivityRequest::AwaitingAnswer,
            serde_json::json!({
                "session_id": "codex-session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "request_user_input"
            }),
        ),
        (
            HookProvider::Claude,
            ProviderLifecycleProvider::Claude,
            ProviderActivityRequest::Working,
            serde_json::json!({
                "session_id": "claude-session-1",
                "hook_event_name": "UserPromptSubmit"
            }),
        ),
        (
            HookProvider::Claude,
            ProviderLifecycleProvider::Claude,
            ProviderActivityRequest::Working,
            serde_json::json!({
                "session_id": "claude-session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash"
            }),
        ),
        (
            HookProvider::Codex,
            ProviderLifecycleProvider::Codex,
            ProviderActivityRequest::Working,
            serde_json::json!({
                "session_id": "codex-session-1",
                "hook_event_name": "UserPromptSubmit"
            }),
        ),
        (
            HookProvider::Codex,
            ProviderLifecycleProvider::Codex,
            ProviderActivityRequest::Working,
            serde_json::json!({
                "session_id": "codex-session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash"
            }),
        ),
    ];

    for (provider, _, _, payload) in &cases {
        assert_eq!(
            receive_from(Cursor::new(serde_json::to_vec(payload).unwrap()), *provider,).unwrap(),
            "{}"
        );
    }

    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), cases.len());
    for (request, (_, expected_provider, expected_activity, _)) in recorded.iter().zip(cases) {
        assert_eq!(request.provider, expected_provider);
        let ProviderLifecycleSignalRequest::ActivityObserved { activity, .. } = &request.signal
        else {
            panic!("activity event must reach the local API as ActivityObserved");
        };
        assert_eq!(*activity, expected_activity);
    }
    server.shutdown();
}
