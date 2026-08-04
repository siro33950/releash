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
fn test_hook受信_live_local_apiを通してsignalを永続化する() {
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
    let scope = ProviderLifecycleScope::new("agent-1", "workflow-1", "node-1", 1).unwrap();
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
    let _workflow_execution_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_WORKFLOW_EXECUTION_ID",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_WORKFLOW_EXECUTION_ID"),
    );
    let _node_execution_id = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_NODE_EXECUTION_ID",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_NODE_EXECUTION_ID"),
    );
    let _attempt = EnvVarGuard::set_value(
        "RELEASH_PROVIDER_LIFECYCLE_ATTEMPT",
        launch_value("RELEASH_PROVIDER_LIFECYCLE_ATTEMPT"),
    );
    let payload = br#"{
        "session_id":"claude-session-1",
        "transcript_path":"provider://claude/transcript",
        "cwd":"/workspace",
        "hook_event_name":"SessionStart",
        "source":"startup"
    }"#;

    receive_from(Cursor::new(payload), HookProvider::Claude).unwrap();

    let page = runtime
        .block_on(store.load_stream(LoadStreamRequest {
            stream_id: StreamId::agent_session("agent-1").unwrap(),
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
    assert_eq!(provider_events, 2);

    server.shutdown();
}
