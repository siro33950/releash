use super::*;
use crate::adaptor::gateway::local_event_store::{
    layout::StoreLayout, node_events::NewNodeEventRow, LocalEventStoreConfig,
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

#[test]
fn test_起動失敗abort_読取後の追記を検出し最新の終端状態を保持する() {
    for concurrent in [
        NodeFact::ExecutionCompleted,
        NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
            reason: Some("user abort".into()),
        }),
        NodeFact::StopReceived(crate::domain::workflow::StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
    ] {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let root =
            SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
                .unwrap();
        fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
        let repository = StoredWorkflowStartupRepository(store.clone());
        let mut stale = repository.load("tree").unwrap().unwrap();
        let fact = stale
            .execution
            .abort_with_reason("advancement failed".into(), 3.0)
            .unwrap();

        // When
        fact_log::append_single_fact(&store, &root.meta, &concurrent, 2_000).unwrap();
        let before = fact_log::read_tree_records(&store, "tree").unwrap();
        let result = repository.append(&stale.root, &fact, 3.0, Some(stale.head));

        // Then
        assert_eq!(stale.head, 1);
        assert!(matches!(result, Err(WorkflowError::Conflict(_))));
        assert_eq!(fact_log::read_tree_records(&store, "tree").unwrap(), before);
        let mut latest = repository.load("tree").unwrap().unwrap();
        assert_eq!(latest.head, 2);
        let abort = latest
            .execution
            .abort_with_reason("advancement failed".into(), 3.0);
        assert_eq!(abort.is_some(), concurrent.terminal_state().is_none());
        if let Some(fact) = abort {
            repository
                .append(&latest.root, &fact, 3.0, Some(latest.head))
                .unwrap();
            assert_eq!(
                fact_log::read_tree_records(&store, "tree")
                    .unwrap()
                    .last()
                    .unwrap()
                    .fact,
                fact
            );
        } else {
            assert_eq!(fact_log::read_tree_records(&store, "tree").unwrap(), before);
        }
    }
}

#[test]
fn test_起動時abort_結果不明でも保存済みなら成功し重複追記しない() {
    for expected_head in [Some(1), None] {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let root =
            SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
                .unwrap();
        fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
        let repository = StoredWorkflowStartupRepository(store.clone());
        let fact = NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
            reason: Some("startup failed".into()),
        });
        store.fault_injector().arm_drop_reply();

        // When
        repository
            .append(&root.meta, &fact, 3.0, expected_head)
            .unwrap();
        store.close_write_queue_for_tests();
        repository
            .append(&root.meta, &fact, 3.0, expected_head)
            .unwrap();

        // Then
        let records = fact_log::read_tree_records(&store, "tree").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].fact, fact);
        assert_eq!(
            repository.load("tree").unwrap().unwrap().execution.state(),
            &RuntimeExecutionState::Aborted
        );
    }
}

#[test]
fn test_起動時abort_未保存や別内容の結果不明は再評価を要求する() {
    for expected_head in [Some(1), None] {
        for change in ["missing", "reason", "timestamp", "node", "attempt"] {
            // Given
            let dir = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into()))
                .unwrap();
            let root = SessionExecutionTreeRootFacts::new(
                "tree",
                "/repo",
                "/repo",
                ProviderKind::Codex,
                None,
            )
            .unwrap();
            fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
            let fact = NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
                reason: Some("startup failed".into()),
            });
            let mut pending = fact_log::pending_single_fact(&root.meta, &fact, 3_000).unwrap();
            match change {
                "reason" => pending.row.detail = r#"{"reason":"user abort"}"#.into(),
                "timestamp" => pending.timestamp_ms += 1,
                "node" => pending.row.node_execution_id = "other-node".into(),
                "attempt" => pending.row.attempt += 1,
                "missing" => {}
                _ => unreachable!(),
            }
            if change != "missing" {
                store
                    .append_node_event_blocking(pending.row, Some(pending.timestamp_ms))
                    .unwrap();
            }
            let before = fact_log::read_tree_records(&store, "tree").unwrap();
            store.close_write_queue_for_tests();

            // When
            let result = StoredWorkflowStartupRepository(store.clone()).append(
                &root.meta,
                &fact,
                3.0,
                expected_head,
            );

            // Then
            assert!(
                matches!(result, Err(WorkflowError::Conflict(_))),
                "{change}: {result:?}"
            );
            assert_eq!(fact_log::read_tree_records(&store, "tree").unwrap(), before);
        }
    }
}

#[test]
fn test_起動時abort_結果不明の読戻し失敗を成功や競合にしない() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000).unwrap();
    let repository = StoredWorkflowStartupRepository(store.clone());
    let stall = store.fault_injector().arm_node_event_append_stall();
    store.fault_injector().arm_drop_reply();
    let append = std::thread::spawn(move || {
        repository.append(
            &root.meta,
            &NodeFact::AbortRequested(Default::default()),
            3.0,
            Some(1),
        )
    });
    stall.wait_until_arrived();

    // When
    let connection =
        rusqlite::Connection::open(StoreLayout::new(dir.path()).database_path()).unwrap();
    connection
        .execute_batch("ALTER TABLE node_events RENAME TO unavailable_node_events;")
        .unwrap();
    stall.release();
    let error = append.join().unwrap().unwrap_err();

    // Then
    assert!(!matches!(error, WorkflowError::Conflict(_)));
    assert!(error.to_string().contains("startup abort readback failed"));
}
