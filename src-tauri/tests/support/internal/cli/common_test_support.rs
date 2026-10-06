use std::fs;
use std::path::Path;

use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::domain::workflow::WorkflowExecutionSummary as WorkflowExecutionMetadata;

pub(in crate::cli) fn write_review_config(data_dir: &Path) {
    fs::write(data_dir.join("releash.toml"), "[agents.codex]\n").unwrap();
}

pub(in crate::cli) async fn write_review_session(
    data_dir: &Path,
    session_id: &str,
    backend_id: Option<&str>,
) {
    write_review_session_with_lifecycle(
        data_dir,
        session_id,
        backend_id,
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open,
    )
    .await;
}

pub(in crate::cli) async fn write_review_session_with_lifecycle(
    data_dir: &Path,
    session_id: &str,
    backend_id: Option<&str>,
    lifecycle: crate::domain::agent_session::aggregates::AgentSessionLifecycle,
) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .unwrap();
    let provider = match backend_id.unwrap_or("codex") {
        "claude" => crate::domain::provider_lifecycle::ProviderKind::Claude,
        _ => crate::domain::provider_lifecycle::ProviderKind::Codex,
    };
    let root_facts = crate::domain::workflow::SessionExecutionTreeRootFacts::new(
        session_id, "/repo", "/repo", provider, None,
    )
    .unwrap();
    let meta = root_facts.meta.clone();
    crate::adaptor::gateway::workflow::fact_log::append_fact_batch_for_seed(
        &store,
        &root_facts.into_facts(),
        1,
        &format!("cli-review-session-{session_id}"),
    )
    .unwrap();
    let lifecycle_fact = match lifecycle {
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open => None,
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Paused => {
            Some(crate::domain::workflow::NodeFact::ProcessExited(
                crate::domain::workflow::ProcessExitedFact {
                    exit_code: Some(0),
                    result_summary: None,
                    failure_reason: None,
                    failure_kind: None,
                },
            ))
        }
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Archived => {
            Some(crate::domain::workflow::NodeFact::ArchiveRequested(
                crate::domain::workflow::ArchiveRequestedFact {
                    reason: "manual".into(),
                    archived_at: 0.0,
                },
            ))
        }
    };
    if let Some(fact) = lifecycle_fact {
        crate::adaptor::gateway::workflow::fact_log::append_single_fact(&store, &meta, &fact, 3)
            .await
            .unwrap();
    }
}

pub(in crate::cli) fn initialize_canonical_store(data_dir: &Path) {
    drop(
        LocalEventStore::open(LocalEventStoreConfig::production(
            data_dir.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("initialize canonical local event store"),
    );
}

pub(in crate::cli) async fn append_workflow_event(data_dir: &Path, event: &WorkflowEvent) {
    append_workflow_events(data_dir, std::slice::from_ref(event)).await;
}

pub(in crate::cli) async fn append_workflow_events(data_dir: &Path, events: &[WorkflowEvent]) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .expect("open canonical local event store");
    crate::adaptor::gateway::workflow::fact_log::append_facts_for_events(&store, events)
        .await
        .expect("append canonical node fact fixture");
}

pub(in crate::cli) async fn write_canonical_execution(
    data_dir: &Path,
    execution: &WorkflowExecutionMetadata,
) {
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.to_path_buf(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .expect("open canonical local event store");
    crate::adaptor::gateway::workflow::test_support::seed_canonical_execution(
        &store,
        execution,
        &[],
    )
    .await;
}

pub(in crate::cli) use super::test_helpers_common::{
    execution_started_event, make_execution, root_node_started_event, test_uuid,
};
