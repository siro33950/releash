use super::*;
use crate::adaptor::gateway::local_event_store::{
    node_events::NewNodeEventRow, LocalEventStoreConfig,
};
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{RuntimeExecutionState, SessionExecutionTreeRootFacts};

#[test]
fn test_起動時判定_rootと最初の終端だけで復元し通常の事実をdecodeしない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
    store
        .append_node_event_blocking(
            NewNodeEventRow {
                tree_id: "tree".into(),
                node_execution_id: "tree".into(),
                parent_id: None,
                node_name: "session".into(),
                kind: "session".into(),
                attempt: 1,
                event_type: "submit_received".into(),
                session_id: None,
                detail: "{".into(),
            },
            Some(2_000),
        )
        .unwrap();
    let retried_root = NodeFactMeta {
        node_execution_id: "retried-root".into(),
        attempt: 2,
        ..root.meta.clone()
    };
    fact_log::append_single_fact(&store, &retried_root, &NodeFact::ExecutionCompleted, 3_000)
        .unwrap();
    fact_log::append_single_fact(
        &store,
        &root.meta,
        &NodeFact::AbortRequested(Default::default()),
        4_000,
    )
    .unwrap();
    let repository = StoredWorkflowStartupRepository(store);

    // When
    let record = repository.load("tree").unwrap().unwrap();

    // Then
    assert_eq!(record.execution.state(), &RuntimeExecutionState::Completed);
    assert_eq!(record.execution.updated_at, 3.0);
    assert!(record.definition_error.is_none());
    assert!(record.execution.workflow.is_none());
    assert_eq!(repository.list_tree_ids().unwrap(), ["tree"]);
    assert!(repository.load("missing").unwrap().is_none());
}

#[test]
fn test_起動時判定_壊れた終端を読取失敗として返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
    store
        .append_node_event_blocking(
            NewNodeEventRow {
                tree_id: "tree".into(),
                node_execution_id: "tree".into(),
                parent_id: None,
                node_name: "session".into(),
                kind: "session".into(),
                attempt: 1,
                event_type: "abort_requested".into(),
                session_id: None,
                detail: "{".into(),
            },
            Some(2_000),
        )
        .unwrap();
    // When / Then
    assert!(StoredWorkflowStartupRepository(store).load("tree").is_err());
}
