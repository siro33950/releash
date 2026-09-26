use super::*;
use crate::adaptor::gateway::local_event_store::{
    node_events::NewNodeEventRow, LocalEventStoreConfig,
};
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::NodeFactMeta;
use crate::domain::workflow::{RuntimeExecutionState, SessionExecutionTreeRootFacts};

#[tokio::test]
async fn test_起動時判定_rootと最初の終端だけで復元し通常の事実をdecodeしない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
        .await
        .unwrap();
    store
        .append_node_event(
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
        .await
        .unwrap();
    let retried_root = NodeFactMeta {
        node_execution_id: "retried-root".into(),
        attempt: 2,
        ..root.meta.clone()
    };
    fact_log::append_single_fact(&store, &retried_root, &NodeFact::ExecutionCompleted, 3_000)
        .await
        .unwrap();
    fact_log::append_single_fact(
        &store,
        &root.meta,
        &NodeFact::AbortRequested(Default::default()),
        4_000,
    )
    .await
    .unwrap();
    let repository = StoredWorkflowStartupRepository(store);

    // When
    let record = repository.load("tree").await.unwrap().unwrap();

    // Then
    assert_eq!(record.execution.state(), &RuntimeExecutionState::Completed);
    assert_eq!(record.execution.updated_at, 3.0);
    assert!(record.definition_error.is_none());
    assert!(record.execution.workflow.is_none());
    assert_eq!(repository.list_tree_ids().await.unwrap(), ["tree"]);
    assert!(repository.load("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn test_起動時判定_壊れた終端を読取失敗として返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
        .await
        .unwrap();
    store
        .append_node_event(
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
        .await
        .unwrap();
    // When / Then
    assert!(StoredWorkflowStartupRepository(store)
        .load("tree")
        .await
        .is_err());
}

#[tokio::test]
async fn test_起動時判定_読取後に追記された最新の終端状態を読み直す() {
    for concurrent in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(Default::default()),
        NodeFact::StopReceived(crate::domain::workflow::StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let root =
            SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
                .unwrap();
        fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
            .await
            .unwrap();
        let repository = StoredWorkflowStartupRepository(store.clone());
        assert!(repository
            .load("tree")
            .await
            .unwrap()
            .unwrap()
            .execution
            .is_active());
        fact_log::append_single_fact(&store, &root.meta, &concurrent, 2_000)
            .await
            .unwrap();
        let before = fact_log::read_tree_records(&store, "tree").await.unwrap();
        crate::usecase::workflow::startup::check_startup_definition(&repository, "tree")
            .await
            .unwrap();
        let latest = repository.load("tree").await.unwrap().unwrap();
        assert_eq!(
            latest.execution.is_active(),
            concurrent.terminal_state().is_none()
        );
        assert_eq!(
            fact_log::read_tree_records(&store, "tree").await.unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn test_起動時判定_書込口を閉じても保存済み終端を読み重複追記しない() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
        .await
        .unwrap();
    fact_log::append_single_fact(
        &store,
        &root.meta,
        &NodeFact::AbortRequested(Default::default()),
        2_000,
    )
    .await
    .unwrap();
    let before = fact_log::read_tree_records(&store, "tree").await.unwrap();
    store.close_write_queue_for_tests();
    let repository = StoredWorkflowStartupRepository(store.clone());
    for _ in 0..2 {
        crate::usecase::workflow::startup::check_startup_definition(&repository, "tree")
            .await
            .unwrap();
        assert_eq!(
            repository
                .load("tree")
                .await
                .unwrap()
                .unwrap()
                .execution
                .state(),
            &RuntimeExecutionState::Aborted
        );
        assert_eq!(
            fact_log::read_tree_records(&store, "tree").await.unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn test_起動時判定_書込口を閉じても未終端と異なる終端の内容を変更しない() {
    for terminal in [
        None,
        Some(NodeFact::ExecutionCompleted),
        Some(NodeFact::AbortRequested(
            crate::domain::workflow::AbortRequestedFact {
                reason: Some("user abort".into()),
            },
        )),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let root =
            SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
                .unwrap();
        fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
            .await
            .unwrap();
        if let Some(fact) = &terminal {
            fact_log::append_single_fact(&store, &root.meta, fact, 2_000)
                .await
                .unwrap();
        }
        let before = fact_log::read_tree_records(&store, "tree").await.unwrap();
        store.close_write_queue_for_tests();
        let repository = StoredWorkflowStartupRepository(store.clone());
        crate::usecase::workflow::startup::check_startup_definition(&repository, "tree")
            .await
            .unwrap();
        assert_eq!(
            repository
                .load("tree")
                .await
                .unwrap()
                .unwrap()
                .execution
                .is_active(),
            terminal.is_none()
        );
        assert_eq!(
            fact_log::read_tree_records(&store, "tree").await.unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn test_起動時判定_読取失敗を成功や競合に変換しない() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use connectrpc::ErrorCode;
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    store.fail_next_read(ReadFailure::Query(
        crate::domain::local_event::LocalEventQueryError::QueryBusy,
    ));
    let error = StoredWorkflowStartupRepository(store)
        .load("tree")
        .await
        .err()
        .unwrap();
    assert_eq!(error.connect_code(), ErrorCode::Unavailable);
    assert!(!matches!(error, WorkflowError::Conflict(_)));
}

#[tokio::test]
async fn test_起動時判定_存在しない実行木の確認で追記しない() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let repository = StoredWorkflowStartupRepository(store);
    crate::usecase::workflow::startup::check_startup_definition(&repository, "missing")
        .await
        .unwrap();
    assert!(repository.list_tree_ids().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_起動時読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::presenter::connect::classified_error;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let repository = StoredWorkflowStartupRepository(store.clone());
    for (failure, expected) in ReadFailure::cases() {
        store.fail_next_read(failure);
        // When
        let error = repository.load("tree").await.err().expect("read must fail");
        // Then
        assert_eq!(classified_error(error).code, expected);
    }
}
