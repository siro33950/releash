use crate::adaptor::gateway::workflow::event::WorkflowEvent;

fn managed_store(
    app: &super::workflow_host::WorkflowRuntimeDependencies,
) -> Result<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>, String> {
    app.store
        .clone()
        .ok_or_else(|| "workflow SQLite event authority is not managed".to_string())
}

/// engine の必須 event 列を統一 Node 事実ログ（node_events）へ追記する。
///
/// 実行木の完了を含む列は原子的に保存する。
pub(crate) fn append_required_events_for_app(
    app: &super::workflow_host::WorkflowRuntimeDependencies,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let store = managed_store(app)?;
    crate::adaptor::gateway::workflow::fact_log::append_facts_for_events(&store, events)
}

/// provider Stop の受理: 事実（stop_received 等）を先に追記し、provider lifecycle
/// event を provider ストリームへ commit する。
///
/// workflow facts の確定後に provider lifecycle を確定する。
pub(crate) async fn commit_provider_stop_for_app(
    app: &super::workflow_host::WorkflowRuntimeDependencies,
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
) -> Result<(), String> {
    let store = managed_store(app)?;
    let installation_id = store.installation_id().to_string();
    commit_provider_lifecycle_events(store, installation_id, provider_events).await
}

async fn commit_provider_lifecycle_events(
    repository: std::sync::Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    installation_id: String,
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
) -> Result<(), String> {
    use sha2::Digest;
    let occurred_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .map_err(|error| format!("provider lifecycle clock is before UNIX_EPOCH: {error}"))?;
    let mut uncommitted = Vec::with_capacity(provider_events.len());
    let mut expected_heads: Vec<crate::domain::local_event::ExpectedStreamHead> = Vec::new();
    for scoped in provider_events {
        let (scope, event) = scoped.into_parts();
        let provider_stream =
            crate::domain::local_event::StreamId::provider_lifecycle(scope.agent_session_id())
                .map_err(|_| "provider lifecycle stream identity is invalid".to_string())?;
        if !expected_heads
            .iter()
            .any(|expected| expected.stream_id == provider_stream)
        {
            let page = repository
                .load_stream(crate::domain::local_event::LoadStreamRequest {
                    stream_id: provider_stream.clone(),
                    after: None,
                    limit: 1,
                })
                .await
                .map_err(|error| format!("provider lifecycle SQLite head read failed: {error}"))?;
            expected_heads.push(crate::domain::local_event::ExpectedStreamHead {
                stream_id: provider_stream.clone(),
                expected: page.head,
            });
        }
        uncommitted.push(crate::domain::local_event::UncommittedDomainEvent {
            stream_id: provider_stream,
            event: crate::domain::local_event::LocalDomainEvent::ProviderLifecycle(event),
            occurred_at_ms,
        });
    }
    let identity_bytes = repository.canonical_event_batch_identity_v1(&uncommitted)?;
    let payload_hash: [u8; 32] = sha2::Sha256::digest(&identity_bytes).into();
    let identity = format!("workflow-provider-stop-{}", hex::encode(payload_hash));
    let batch = crate::domain::local_event::LocalAtomicBatch {
        commit_id: crate::domain::local_event::CommitIdentity::parse(&identity)
            .map_err(|_| "provider stop commit identity is invalid".to_string())?,
        idempotency: crate::domain::local_event::IdempotencyBinding {
            installation_id,
            operation_kind: crate::domain::local_event::CommitOperationKind::Workflow,
            idempotency_key: hex::encode(payload_hash),
            payload_hash,
        },
        expected_heads,
        events: uncommitted,
        state_mutations: Vec::new(),
    };
    repository
        .commit_batch(batch)
        .await
        .map(|_| ())
        .map_err(|error| format!("provider lifecycle SQLite commit failed: {error}"))
}
