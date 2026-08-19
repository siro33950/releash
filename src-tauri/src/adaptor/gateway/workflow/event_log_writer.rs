use tauri::Manager;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;

fn managed_store<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>, String> {
    app.try_state::<std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>()
        .map(|store| store.inner().clone())
        .ok_or_else(|| "workflow SQLite event authority is not managed".to_string())
}

/// engine の必須 event 列を統一 Node 事実ログ（node_events）へ追記する。
///
/// 原子性依存は行単位の append のみ（純粋事実ログの規約）。導出表・遷移 event の
/// 永続化は存在しない。
pub(crate) fn append_required_events_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
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
/// 事実が先である理由: 途中で落ちた場合、事実側が残れば node は完了し（利用者の
/// 観測として正しい）、provider 側の再配送は同じ事実の重複 append（fold で無害）に
/// 収束する。逆順は「provider は止まったが node が完了しない」窓を作る。
pub(crate) fn append_provider_stop_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    events: &[WorkflowEvent],
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
) -> Result<(), String> {
    let store = managed_store(app)?;
    crate::adaptor::gateway::workflow::fact_log::append_facts_for_events(&store, events)?;
    if provider_events.is_empty() {
        return Ok(());
    }
    commit_provider_lifecycle_events_blocking(&store, provider_events)
}

fn commit_provider_lifecycle_events_blocking(
    store: &std::sync::Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
) -> Result<(), String> {
    let repository: std::sync::Arc<
        dyn crate::domain::local_event::LocalEventTransactionRepository,
    > = store.clone();
    let installation_id = store.installation_id().to_string();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| format!("failed to create provider commit runtime: {error}"))?
                    .block_on(commit_provider_lifecycle_events(
                        repository,
                        installation_id,
                        provider_events,
                    ))
            })
            .join()
            .map_err(|_| "provider lifecycle commit worker panicked".to_string())?
    })
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
        .unwrap_or(0);
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
