use std::fs;
use std::path::Path;

use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::workflow::WorkflowEvent;
use releash_lib::test_support::integration::workflow::WorkflowExecutionSummary as WorkflowExecutionMetadata;

pub(crate) fn write_review_config(data_dir: &Path) {
    fs::write(data_dir.join("releash.toml"), "[agents.codex]\n").unwrap();
}

pub(crate) async fn write_review_session(
    data_dir: &Path,
    session_id: &str,
    backend_id: Option<&str>,
) {
    write_review_session_with_lifecycle(
        data_dir,
        session_id,
        backend_id,
        releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Open,
    )
    .await;
}

pub(crate) async fn write_review_session_with_lifecycle(
    data_dir: &Path,
    session_id: &str,
    backend_id: Option<&str>,
    lifecycle: releash_lib::test_support::integration::sessions::AgentSessionLifecycle,
) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let provider = match backend_id.unwrap_or("codex") {
        "claude" => releash_lib::test_support::integration::providers::ProviderKind::Claude,
        _ => releash_lib::test_support::integration::providers::ProviderKind::Codex,
    };
    let root_facts =
        releash_lib::test_support::integration::workflow::SessionExecutionTreeRootFacts::new(
            session_id, "/repo", "/repo", provider, None,
        )
        .unwrap();
    let meta = root_facts.meta.clone();
    releash_lib::test_support::integration::workflow::append_fact_batch_for_seed(
        &store,
        &root_facts.into_facts(),
        1,
        &format!("cli-review-session-{session_id}"),
    )
    .unwrap();
    let lifecycle_fact = match lifecycle {
        releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Open => None,
        releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Paused => Some(
            releash_lib::test_support::integration::workflow::NodeFact::ProcessExited(
                releash_lib::test_support::integration::workflow::ProcessExitedFact {
                    exit_code: Some(0),
                    result_summary: None,
                    failure_reason: None,
                    failure_kind: None,
                },
            ),
        ),
        releash_lib::test_support::integration::sessions::AgentSessionLifecycle::Archived => Some(
            releash_lib::test_support::integration::workflow::NodeFact::ArchiveRequested(
                releash_lib::test_support::integration::workflow::ArchiveRequestedFact {
                    reason: "manual".into(),
                    archived_at: 0.0,
                },
            ),
        ),
    };
    if let Some(fact) = lifecycle_fact {
        releash_lib::test_support::integration::workflow::append_single_fact(
            &store, &meta, &fact, 3,
        )
        .await
        .unwrap();
    }
}

pub(crate) fn initialize_canonical_store(data_dir: &Path) {
    drop(
        LocalEventStore::open(LocalEventStoreConfig::production(
            data_dir.to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ))
        .expect("initialize canonical local event store"),
    );
}

pub(crate) async fn append_workflow_event(data_dir: &Path, event: &WorkflowEvent) {
    append_workflow_events(data_dir, std::slice::from_ref(event)).await;
}

pub(crate) async fn append_workflow_events(data_dir: &Path, events: &[WorkflowEvent]) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .expect("open canonical local event store");
    releash_lib::test_support::integration::workflow::append_facts_for_events(&store, events)
        .await
        .expect("append canonical node fact fixture");
}

pub(crate) async fn write_canonical_execution(
    data_dir: &Path,
    execution: &WorkflowExecutionMetadata,
) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RetryLimiter::new()),
    ))
    .expect("open canonical local event store");
    releash_lib::test_support::integration::workflow::seed_canonical_execution(
        &store,
        execution,
        &[],
    )
    .await;
}
