use super::*;
use crate::adaptor::gateway::local_event_store::{
    layout::StoreLayout, node_events::NewNodeEventRow, LocalEventStoreConfig,
};
use crate::domain::provider_lifecycle::ProviderKind;
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
async fn test_起動失敗abort_読取後の追記を検出し最新の終端状態を保持する() {
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
        fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
            .await
            .unwrap();
        let repository = StoredWorkflowStartupRepository(store.clone());
        let mut stale = repository.load("tree").await.unwrap().unwrap();
        let fact = stale
            .execution
            .abort_with_reason("advancement failed".into(), 3.0)
            .unwrap();

        // When
        fact_log::append_single_fact(&store, &root.meta, &concurrent, 2_000)
            .await
            .unwrap();
        let before = fact_log::read_tree_records(&store, "tree").await.unwrap();
        let result = repository
            .append(&stale.root, &fact, 3.0, Some(&stale.revision))
            .await;

        // Then
        assert_eq!(stale.revision, WorkflowRevision("1".into()));
        assert!(matches!(result, Err(WorkflowError::Conflict(_))));
        assert_eq!(
            fact_log::read_tree_records(&store, "tree").await.unwrap(),
            before
        );
        let mut latest = repository.load("tree").await.unwrap().unwrap();
        assert_eq!(latest.revision, WorkflowRevision("2".into()));
        let abort = latest
            .execution
            .abort_with_reason("advancement failed".into(), 3.0);
        assert_eq!(abort.is_some(), concurrent.terminal_state().is_none());
        if let Some(fact) = abort {
            repository
                .append(&latest.root, &fact, 3.0, Some(&latest.revision))
                .await
                .unwrap();
            assert_eq!(
                fact_log::read_tree_records(&store, "tree")
                    .await
                    .unwrap()
                    .last()
                    .unwrap()
                    .fact,
                fact
            );
        } else {
            assert_eq!(
                fact_log::read_tree_records(&store, "tree").await.unwrap(),
                before
            );
        }
    }
}

#[tokio::test]
async fn test_起動時abort_結果不明でも保存済みなら成功し重複追記しない() {
    for expected_head in [Some(WorkflowRevision("1".into())), None] {
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
        let repository = StoredWorkflowStartupRepository(store.clone());
        let fact = NodeFact::AbortRequested(crate::domain::workflow::AbortRequestedFact {
            reason: Some("startup failed".into()),
        });
        store.fault_injector().arm_drop_reply();

        // When
        repository
            .append(&root.meta, &fact, 3.0, expected_head.as_ref())
            .await
            .unwrap();
        store.close_write_queue_for_tests();
        repository
            .append(&root.meta, &fact, 3.0, expected_head.as_ref())
            .await
            .unwrap();

        // Then
        let records = fact_log::read_tree_records(&store, "tree").await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].fact, fact);
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
    }
}

#[tokio::test]
async fn test_起動時abort_未保存や別内容の結果不明は再評価を要求する() {
    for expected_head in [Some(WorkflowRevision("1".into())), None] {
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
            fact_log::append_single_fact(&store, &root.meta, &root.started, 1_000)
                .await
                .unwrap();
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
                    .append_node_event(pending.row, Some(pending.timestamp_ms))
                    .await
                    .unwrap();
            }
            let before = fact_log::read_tree_records(&store, "tree").await.unwrap();
            store.close_write_queue_for_tests();

            // When
            let result = StoredWorkflowStartupRepository(store.clone())
                .append(&root.meta, &fact, 3.0, expected_head.as_ref())
                .await;

            // Then
            assert!(
                matches!(result, Err(WorkflowError::Conflict(_))),
                "{change}: {result:?}"
            );
            assert_eq!(
                fact_log::read_tree_records(&store, "tree").await.unwrap(),
                before
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_起動時abort_結果不明の読戻し失敗を成功や競合にしない() {
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
    let repository = StoredWorkflowStartupRepository(store.clone());
    let stall = store.fault_injector().arm_node_event_append_stall();
    store.fault_injector().arm_drop_reply();
    let append = tokio::spawn(async move {
        repository
            .append(
                &root.meta,
                &NodeFact::AbortRequested(Default::default()),
                3.0,
                Some(&WorkflowRevision("1".into())),
            )
            .await
    });
    stall.wait_until_arrived();

    // When
    let connection =
        rusqlite::Connection::open(StoreLayout::new(dir.path()).database_path()).unwrap();
    connection
        .execute_batch("ALTER TABLE node_events RENAME TO unavailable_node_events;")
        .unwrap();
    stall.release();
    let error = append.await.unwrap().unwrap_err();

    // Then
    assert!(!matches!(error, WorkflowError::Conflict(_)));
    assert!(error.to_string().contains("startup abort readback failed"));
}

#[tokio::test]
async fn test_起動時abort_不正なrevisionは追記前に拒否する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root =
        SessionExecutionTreeRootFacts::new("tree", "/repo", "/repo", ProviderKind::Codex, None)
            .unwrap();
    fact_log::append_single_fact(&store, &root.meta, &root.started, 1000)
        .await
        .unwrap();
    let repository = StoredWorkflowStartupRepository(store.clone());
    // When
    let result = repository
        .append(
            &root.meta,
            &NodeFact::AbortRequested(Default::default()),
            2.0,
            Some(&WorkflowRevision("invalid".into())),
        )
        .await;
    // Then
    assert!(result.is_err());
    assert_eq!(
        fact_log::read_tree_records(&store, "tree")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn test_起動時読取_実経路で失敗分類を保持する() {
    use crate::adaptor::gateway::local_event_store::test_helpers::ReadFailure;
    use crate::adaptor::protocol::connect::classified_error;
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
