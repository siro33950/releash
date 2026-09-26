use crate::adaptor::gateway::workflow::fact_codec;
use std::collections::HashMap;

use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::workflow::services::fact_replay::fold_execution_tree;
use crate::domain::workflow::value_objects::ContractViolationRecord;
use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, ExecutionParentRef, NodeDefinition, NodeKind,
    RuntimeExecutionState, SequenceSpec, SessionExecutionTreeRootFacts, WorkflowDefinition,
};

const TREE: &str = "00000000-0000-4000-8000-00000000e001";

fn definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![
            NodeDefinition {
                name: "a".to_string(),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "run".to_string(),
                kind: NodeKind::Command(crate::domain::workflow::CommandSpec {
                    command: "true".to_string(),
                    env: [(
                        crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap(),
                        crate::domain::workflow::InputParameterRef::new("document").unwrap(),
                    )]
                    .into_iter()
                    .collect(),
                }),
                input: vec![crate::domain::workflow::InputParam {
                    name: "document".to_string(),
                    contract: None,
                }],
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Sequence(SequenceSpec {
                    entry: None,
                    children: vec![ChildEntry::reference("a"), ChildEntry::reference("run")],
                }),
                ..NodeDefinition::default()
            },
        ],
        entry: "main".to_string(),
    }
}

fn started_event() -> WorkflowEvent {
    WorkflowEvent::ExecutionStarted {
        repository_root: None,
        execution_id: TREE.to_string(),
        workflow_name: "wf".to_string(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Cli,
        request: "please".to_string(),
        definition: definition(),
        timestamp: 1.0,
    }
}

fn node_started(
    node_execution_id: &str,
    node_name: &str,
    kind: NodeKindName,
    parent: Option<ExecutionParentRef>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::NodeStarted {
        worktree: None,
        execution_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        kind,
        attempt: 1,
        parent,
        timestamp,
    }
}

fn no_lookup(_: &str) -> Result<Option<FactRowMeta>, FactReadError> {
    Ok(None)
}

fn open_fd_count() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}

fn test_fact_meta(tree_id: &str, node_execution_id: &str) -> NodeFactMeta {
    NodeFactMeta {
        tree_id: tree_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        parent_id: None,
        node_name: "main".to_string(),
        kind: NodeKindName::Session,
        attempt: 1,
    }
}

mod fd_invariance_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const FD_TEST_CHILD: &str = "RELEASH_FACT_LOG_FD_TEST_CHILD";

    #[cfg(unix)]
    struct NoFileSoftLimitGuard {
        original: libc::rlimit,
    }

    #[cfg(unix)]
    impl NoFileSoftLimitGuard {
        fn lower_to(soft_limit: usize) -> Self {
            let mut original = std::mem::MaybeUninit::<libc::rlimit>::uninit();
            // SAFETY: getrlimit writes one rlimit value to the valid out pointer.
            let result = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, original.as_mut_ptr()) };
            assert_eq!(result, 0, "failed to read RLIMIT_NOFILE");
            // SAFETY: getrlimit succeeded and initialized the value.
            let original = unsafe { original.assume_init() };
            let soft_limit = soft_limit as libc::rlim_t;
            assert!(
                soft_limit < original.rlim_cur,
                "RLIMIT_NOFILE soft limit is too low to create the test condition"
            );
            let lowered = libc::rlimit {
                rlim_cur: soft_limit,
                rlim_max: original.rlim_max,
            };
            // SAFETY: lowered preserves the inherited hard limit and only lowers the soft limit.
            let result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &lowered) };
            assert_eq!(result, 0, "failed to lower RLIMIT_NOFILE");
            Self { original }
        }
    }

    #[cfg(unix)]
    impl Drop for NoFileSoftLimitGuard {
        fn drop(&mut self) {
            // SAFETY: restores the rlimit value read successfully by lower_to.
            let result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.original) };
            assert_eq!(result, 0, "failed to restore RLIMIT_NOFILE");
        }
    }

    fn run_in_isolated_process(child_name: &str, test_filter: &str) -> bool {
        if std::env::var(FD_TEST_CHILD).as_deref() == Ok(child_name) {
            return false;
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env(FD_TEST_CHILD, child_name)
            .arg(test_filter)
            .arg("--test-threads=1")
            .status()
            .unwrap();
        assert!(status.success());
        true
    }

    fn wait_until_pending_request_count(store: &LocalEventStore, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let actual = store.pending_write_request_count();
            if actual == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "write queue did not reach {expected} pending requests; actual: {actual}"
            );
            std::thread::yield_now();
        }
    }

    #[tokio::test]
    async fn test_事実行追記_単発追記中と完了後にopen_fd数が変わらない() {
        if run_in_isolated_process(
            "single",
            "fd_invariance_tests::test_事実行追記_単発追記中と完了後にopen_fd数が変わらない",
        ) {
            return;
        }

        // Given: INSERT 直前で writer を停止する store と追記前の open fd 数
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let stall = store.fault_injector().arm_node_event_append_stall();
        let meta = test_fact_meta("fd-single-tree", "fd-single-node");
        let before = open_fd_count();

        // When: caller が writer の応答待ちに入った状態で open fd 数を測る
        let worker_store = Arc::clone(&store);
        let worker = append_single_fact(&worker_store, &meta, &NodeFact::RetryRequested, 1_000);
        tokio::pin!(worker);
        assert!(futures_util::poll!(worker.as_mut()).is_pending());
        stall.wait_until_arrived();
        let in_flight = open_fd_count();
        stall.release();
        worker.await.unwrap();
        let after = open_fd_count();

        // Then: 追記中・完了後とも fd 数が増えず、事実行が記録される
        assert_eq!(in_flight, before);
        assert_eq!(after, before);
        let records = read_tree_records(&store, "fd-single-tree").await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].meta.node_execution_id, "fd-single-node");
        assert_eq!(records[0].fact, NodeFact::RetryRequested);
    }

    #[tokio::test]
    async fn test_事実行追記_全追記が並行実行中でもopen_fd数が変わらず全行を記録する() {
        const APPEND_COUNT: usize = 16;

        if run_in_isolated_process(
            "parallel",
            "fd_invariance_tests::test_事実行追記_全追記が並行実行中でもopen_fd数が変わらず全行を記録する",
        ) {
            return;
        }

        // Given: 1本目を INSERT 直前で停止し、同時開始を待つ追記 worker
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let stall = store.fault_injector().arm_node_event_append_stall();
        let mut workers = (0..APPEND_COUNT)
            .map(|index| {
                let store = Arc::clone(&store);
                Box::pin(async move {
                    let meta = test_fact_meta("fd-parallel-tree", &format!("node-{index}"));
                    append_single_fact(&store, &meta, &NodeFact::RetryRequested, index as i64).await
                })
            })
            .collect::<Vec<_>>();
        let before = open_fd_count();

        // When: 1本が writer に到達し、残りすべてが queue に滞留した状態で測る
        assert!(futures_util::poll!(workers[0].as_mut()).is_pending());
        stall.wait_until_arrived();
        for worker in &mut workers[1..] {
            assert!(futures_util::poll!(worker.as_mut()).is_pending());
        }
        wait_until_pending_request_count(&store, APPEND_COUNT - 1);
        let in_flight = open_fd_count();
        stall.release();
        for worker in workers {
            worker.await.unwrap();
        }
        let after = open_fd_count();

        // Then: 全 append の実行中・完了後とも fd 数が増えず、全行が記録される
        assert_eq!(in_flight, before);
        assert_eq!(after, before);
        let records = read_tree_records(&store, "fd-parallel-tree").await.unwrap();
        assert_eq!(records.len(), APPEND_COUNT);
        let mut node_execution_ids = records
            .iter()
            .map(|record| record.meta.node_execution_id.as_str())
            .collect::<Vec<_>>();
        node_execution_ids.sort_unstable();
        let mut expected = (0..APPEND_COUNT)
            .map(|index| format!("node-{index}"))
            .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(node_execution_ids, expected);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_事実行追記_fd_soft_limit直下でもsession_attachedを記録する() {
        if run_in_isolated_process(
            "soft-limit",
            "fd_invariance_tests::test_事実行追記_fd_soft_limit直下でもsession_attachedを記録する",
        ) {
            return;
        }

        // Given: store の fd を確保済みで、soft limit に2個だけ余裕がある子プロセス
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let warm_up_meta = test_fact_meta("fd-warm-up-tree", "fd-warm-up-node");
        append_single_fact(&store, &warm_up_meta, &NodeFact::RetryRequested, 1_000)
            .await
            .unwrap();
        let current_open_fd_count = open_fd_count();
        let _soft_limit = NoFileSoftLimitGuard::lower_to(current_open_fd_count + 2);
        let meta = test_fact_meta("fd-soft-limit-tree", "fd-soft-limit-node");
        let fact = NodeFact::SessionAttached(SessionAttachedFact {
            session_id: "fd-soft-limit-session".to_string(),
            provider_session_id: Some("fd-soft-limit-provider-session".to_string()),
            transcript_ref: Some("fd-soft-limit-transcript".to_string()),
            initial_instruction_admitted: true,
        });

        // When: fd soft limit 直下で session_attached を追記する
        append_single_fact(&store, &meta, &fact, 2_000)
            .await
            .unwrap();

        // Then: fd を追加取得せず追記でき、同じ事実行を既存 reader から読める
        let records = read_tree_records(&store, "fd-soft-limit-tree")
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].meta.node_execution_id, "fd-soft-limit-node");
        assert_eq!(records[0].fact, fact);
    }
}

mod append_contract_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::writer::NORMAL_LANE_MAX_BYTES;

    pub(super) async fn read_raw_rows(
        store: &Arc<LocalEventStore>,
        tree_id: &str,
    ) -> Vec<crate::adaptor::gateway::local_event_store::node_events::NodeEventRow> {
        let tree_id = tree_id.to_string();
        store
            .submit_query(move |connection| {
                node_events::read_tree(connection, &tree_id)
                    .map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_事実行追記_asyncで記録され結果が返る() {
        // Given: async で利用する file-backed store と単独の事実
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let meta = test_fact_meta("sync-context-tree", "sync-context-node");

        // When: 事実行を追記する
        let result = append_single_fact(&store, &meta, &NodeFact::RetryRequested, 1_000).await;

        // Then: 結果が返り、事実行が記録される
        assert_eq!(result, Ok(()));
        let records = read_tree_records(&store, "sync-context-tree")
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fact, NodeFact::RetryRequested);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_事実行追記_async_runtime上でpanicせず記録され結果が返る() {
        // Given: current-thread tokio runtime 上で利用する file-backed store と単独の事実
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let meta = test_fact_meta("async-context-tree", "async-context-node");

        // When: runtime worker 上から async append を呼ぶ
        let result = append_single_fact(&store, &meta, &NodeFact::ResumeRequested, 2_000).await;

        // Then: 呼び出しが停止せず結果が返り、事実行が記録される
        assert_eq!(result, Ok(()));
        let records = read_tree_records(&store, "async-context-tree")
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fact, NodeFact::ResumeRequested);
    }

    #[tokio::test]
    async fn test_事実行追記_同一nodeの内容とseqが入力順に記録される() {
        // Given: 同一 node に順に発生した、全 field を同定できる3つの事実行
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let meta = NodeFactMeta {
            tree_id: "ordering-tree".to_string(),
            node_execution_id: "ordering-node".to_string(),
            parent_id: Some("ordering-parent".to_string()),
            node_name: "worker".to_string(),
            kind: NodeKindName::Session,
            attempt: 2,
        };
        let facts = [
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "session-1".to_string(),
                provider_session_id: Some("provider-session-1".to_string()),
                transcript_ref: Some("transcript-1".to_string()),
                initial_instruction_admitted: true,
            }),
            NodeFact::RetryRequested,
            NodeFact::ResumeRequested,
        ];
        let rows = facts
            .iter()
            .enumerate()
            .map(|(index, fact)| pending_single_fact(&meta, fact, 1_000 + index as i64))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let expected = rows.clone();

        // When: 3行を1行ずつ async append する
        append_pending_rows(&store, rows).await.unwrap();

        // Then: NewNodeEventRow の全 field・timestamp・払い出し seq が入力順と一致する
        let stored = read_raw_rows(&store, "ordering-tree").await;
        assert_eq!(stored.len(), expected.len());
        for (index, (stored, expected)) in stored.iter().zip(expected.iter()).enumerate() {
            assert_eq!(stored.tree_id, expected.row.tree_id);
            assert_eq!(stored.seq, index as i64 + 1);
            assert_eq!(stored.node_execution_id, expected.row.node_execution_id);
            assert_eq!(stored.parent_id, expected.row.parent_id);
            assert_eq!(stored.node_name, expected.row.node_name);
            assert_eq!(stored.kind, expected.row.kind);
            assert_eq!(stored.attempt, expected.row.attempt);
            assert_eq!(stored.event_type, expected.row.event_type);
            assert_eq!(stored.session_id, expected.row.session_id);
            assert_eq!(stored.detail, expected.row.detail);
            assert_eq!(stored.timestamp_ms, expected.timestamp_ms);
        }
    }

    #[tokio::test]
    async fn test_事実行追記_利用不能な追記先の失敗が呼び出し元へ返る() {
        // Given: write queue が閉じた store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        store.close_write_queue_for_tests();
        let meta = test_fact_meta("unavailable-tree", "unavailable-node");

        // When: 事実行を追記する
        let error = append_single_fact(
            &store,
            &meta,
            &NodeFact::AbortRequested(Default::default()),
            1_000,
        )
        .await
        .unwrap_err();

        // Then: 失敗が握りつぶされず呼び出し元へ返り、行は記録されない
        assert_eq!(
            error,
            crate::domain::workflow::WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(
                    crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown
                )
                .with_message("node fact append failed: node event write outcome is unknown")
            )
        );
        assert!(read_raw_rows(&store, "unavailable-tree").await.is_empty());
    }

    #[tokio::test]
    async fn test_事実行追記_複数行の途中失敗で前の行だけが記録される() {
        // Given: 正常行、batch 容量を超える行、未投入で終わる正常行の順の入力
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let before_meta = test_fact_meta("partial-tree", "before-failure");
        let after_meta = test_fact_meta("partial-tree", "after-failure");
        let before = pending_single_fact(&before_meta, &NodeFact::RetryRequested, 1_000).unwrap();
        let failed = PendingFactRow {
            row: NewNodeEventRow {
                tree_id: "partial-tree".to_string(),
                node_execution_id: "failed-row".to_string(),
                parent_id: None,
                node_name: "main".to_string(),
                kind: "session".to_string(),
                attempt: 1,
                event_type: "retry_requested".to_string(),
                session_id: None,
                detail: "x".repeat(NORMAL_LANE_MAX_BYTES),
            },
            timestamp_ms: 2_000,
        };
        let after = pending_single_fact(&after_meta, &NodeFact::ResumeRequested, 3_000).unwrap();

        // When: 3行を順に追記する
        let error = append_pending_rows(&store, vec![before, failed, after])
            .await
            .unwrap_err();

        // Then: 容量拒否が返り、成功済みの1行だけが durable のまま残る
        assert_eq!(
            error,
            crate::domain::workflow::WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(
                    crate::domain::local_event::CommitBatchError::CapacityExceeded
                )
                .with_message("node fact append failed: batch capacity exceeded")
            )
        );
        let stored = read_raw_rows(&store, "partial-tree").await;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].seq, 1);
        assert_eq!(stored[0].node_execution_id, "before-failure");
    }
}

mod mapping_tests {
    use super::*;

    #[test]
    fn test_写像_開始バッチがroot構成つきstarted行になる() {
        // Given: 起動時の required batch 相当のイベント列
        let events = vec![
            started_event(),
            node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
            node_started(
                "a-exec",
                "a",
                NodeKindName::Session,
                Some(ExecutionParentRef::sequence_child("main-exec")),
                1.0,
            ),
        ];

        // When
        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        // Then: ExecutionStarted は root started に融合され、行は2つ
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row.event_type, "started");
        assert_eq!(rows[0].row.node_execution_id, "main-exec");
        assert!(rows[0].row.parent_id.is_none());
        assert!(rows[0].row.detail.contains("\"launchedAs\":\"workflow\""));
        assert_eq!(rows[1].row.parent_id.as_deref(), Some("main-exec"));
        assert!(!rows[1].row.detail.contains("\"launchedAs\""));
    }

    #[test]
    fn test_写像_実行完了だけ終端事実として記録する() {
        // Given: 完了・承認要求・実行完了などの遷移イベント（session の完了含む）
        let mut batch_meta_events = vec![
            node_started("s-exec", "a", NodeKindName::Session, None, 1.0),
            WorkflowEvent::NodeCompleted {
                execution_id: TREE.to_string(),
                node_execution_id: "s-exec".to_string(),
                node_name: "a".to_string(),
                attempt: 1,
                result_summary: Some("done".to_string()),
                token_usage: None,
                timestamp: 2.0,
            },
            WorkflowEvent::ApprovalRequested {
                execution_id: TREE.to_string(),
                node_execution_id: "s-exec".to_string(),
                node_name: "a".to_string(),
                result_summary: None,
                timestamp: 2.0,
            },
        ];
        batch_meta_events.push(WorkflowEvent::ExecutionCompleted {
            execution_id: TREE.to_string(),
            total_token_usage: Default::default(),
            timestamp: 3.0,
        });

        // When
        let rows = fact_rows_for_events(&batch_meta_events, no_lookup, no_lookup).unwrap();

        // Then: 実行木の完了をrootに記録する
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].row.event_type, "execution_completed");
        assert_eq!(rows[1].row.node_execution_id, "s-exec");
        assert_eq!(rows[0].row.event_type, "started");
    }

    #[test]
    fn test_写像_commandの承認要求は結果要約付きの正常終了の事実として記録する() {
        // Given
        let events = vec![
            node_started("c-exec", "run", NodeKindName::Command, None, 1.0),
            WorkflowEvent::ApprovalRequested {
                execution_id: TREE.to_string(),
                node_execution_id: "c-exec".to_string(),
                node_name: "run".to_string(),
                result_summary: Some("exit_code=0".to_string()),
                timestamp: 2.0,
            },
        ];

        // When
        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        // Then
        assert_eq!(rows.len(), 2);
        let row = &rows[1].row;
        assert_eq!(row.node_execution_id, "c-exec");
        assert_eq!(row.event_type, "process_exited");
        assert_eq!(
            fact_codec::decode(&row.event_type, &row.detail).unwrap(),
            NodeFact::ProcessExited(ProcessExitedFact {
                exit_code: Some(0),
                result_summary: Some("exit_code=0".to_string()),
                failure_reason: None,
                failure_kind: None,
            })
        );
    }

    #[test]
    fn test_写像_commandの完了はprocess_exitedになる() {
        // Given: command node の完了
        let events = vec![
            node_started("c-exec", "run", NodeKindName::Command, None, 1.0),
            WorkflowEvent::NodeCompleted {
                execution_id: TREE.to_string(),
                node_execution_id: "c-exec".to_string(),
                node_name: "run".to_string(),
                attempt: 1,
                result_summary: Some("ok".to_string()),
                token_usage: None,
                timestamp: 2.0,
            },
        ];

        // When
        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        // Then
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].row.event_type, "process_exited");
        assert!(rows[1].row.detail.contains("\"exitCode\":0"));
    }

    #[test]
    fn test_写像_sessionのruntime失敗はprocess_exitと別の事実になる() {
        let events = vec![
            node_started("s-exec", "agent", NodeKindName::Session, None, 1.0),
            WorkflowEvent::NodeFailed {
                execution_id: TREE.to_string(),
                node_execution_id: "s-exec".to_string(),
                node_name: "agent".to_string(),
                attempt: 1,
                reason: "activation failed".to_string(),
                failure_kind:
                    crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 2.0,
            },
        ];

        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].row.event_type, "runtime_failure_observed");
        assert!(rows[1].row.detail.contains("activation failed"));
    }

    #[test]
    fn test_写像_commandの起動失敗はprocess喪失ではなくruntime失敗を記録する() {
        // Given
        let events = vec![
            node_started("c-exec", "run", NodeKindName::Command, None, 1.0),
            WorkflowEvent::NodeFailed {
                execution_id: TREE.to_string(),
                node_execution_id: "c-exec".to_string(),
                node_name: "run".to_string(),
                attempt: 1,
                reason: "worktree creation failed".to_string(),
                failure_kind:
                    crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
                timestamp: 2.0,
            },
        ];

        // When
        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        // Then
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].row.event_type, "runtime_failure_observed");
        assert_eq!(
            fact_codec::decode(&rows[1].row.event_type, &rows[1].row.detail).unwrap(),
            NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                reason: "worktree creation failed".to_string(),
                failure_kind:
                    crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
            })
        );
    }

    #[test]
    fn test_写像_合成子の成果と完了は行にならない() {
        let events = vec![
            node_started("fan-exec", "main", NodeKindName::Fanout, None, 1.0),
            WorkflowEvent::ArtifactProduced {
                execution_id: TREE.to_string(),
                node_execution_id: "fan-exec".to_string(),
                node_name: "main".to_string(),
                contract: None,
                value: serde_json::json!([1, 2]),
                request_id: None,
                submitted_at: None,
                timestamp: 2.0,
            },
            WorkflowEvent::NodeCompleted {
                execution_id: TREE.to_string(),
                node_execution_id: "fan-exec".to_string(),
                node_name: "main".to_string(),
                attempt: 1,
                result_summary: None,
                token_usage: None,
                timestamp: 2.0,
            },
        ];

        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.event_type, "started");
    }

    #[test]
    fn test_写像_contract違反はsubmit_rejectedになる() {
        let events = vec![
            node_started("s-exec", "a", NodeKindName::Session, None, 1.0),
            WorkflowEvent::ContractViolated {
                execution_id: TREE.to_string(),
                node_execution_id: "s-exec".to_string(),
                node_name: "a".to_string(),
                violations: vec![ContractViolationRecord {
                    path: "$.x".to_string(),
                    reason: "missing".to_string(),
                }],
                repair_attempt: 1,
                request_id: Some("req".to_string()),
                timestamp: 2.0,
            },
        ];

        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();
        assert_eq!(rows[1].row.event_type, "submit_rejected");
        assert!(rows[1].row.detail.contains("missing"));
    }

    #[test]
    fn test_写像_abortはrootのnodeに紐づくabort_requestedになる() {
        // Given: 既存の tree（root meta は root_lookup で解決される）
        let root_meta = FactRowMeta {
            node_execution_id: "main-exec".to_string(),
            parent_id: None,
            node_name: "main".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
        };
        let events = vec![WorkflowEvent::ExecutionAborted {
            execution_id: TREE.to_string(),
            aborted_node: None,
            timestamp: 9.0,
        }];

        let rows =
            fact_rows_for_events(&events, no_lookup, move |_| Ok(Some(root_meta.clone()))).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.event_type, "abort_requested");
        assert_eq!(rows[0].row.node_execution_id, "main-exec");
    }

    #[test]
    fn test_写像_バッチ外のnodeはmeta_lookupで補完される() {
        // Given: started が過去バッチにある node への submit
        let known: HashMap<&str, FactRowMeta> = HashMap::from([(
            "s-exec",
            FactRowMeta {
                node_execution_id: "s-exec".to_string(),
                parent_id: Some("main-exec".to_string()),
                node_name: "a".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
            },
        )]);
        let events = vec![WorkflowEvent::NodeSubmitReceived {
            execution_id: TREE.to_string(),
            node_execution_id: "s-exec".to_string(),
            timestamp: 5.0,
        }];

        let rows =
            fact_rows_for_events(&events, move |id| Ok(known.get(id).cloned()), no_lookup).unwrap();
        assert_eq!(rows[0].row.event_type, "submit_received");
        assert_eq!(rows[0].row.attempt, 2);
        assert_eq!(rows[0].row.parent_id.as_deref(), Some("main-exec"));
    }
}

mod reconciliation_tests {
    use super::*;
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository;
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
    use crate::domain::workflow::RuntimeExecutionState;
    use crate::domain::workspace_tree::{
        WorkspaceIdentity, WorkspaceNodeStatusClassification, WorkspaceTreeRepository,
    };

    fn open_store() -> (tempfile::TempDir, std::sync::Arc<LocalEventStore>) {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        (root, store)
    }

    fn test_id_source() -> impl FnMut() -> String {
        let mut counter = 0usize;
        move || {
            counter += 1;
            format!("reconciled-{counter}")
        }
    }

    #[tokio::test]
    async fn test_復旧_起動失敗済みleafは再起動せず起動済みprocessの喪失も記録しない() {
        // Given
        let (_root, store) = open_store();
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
                node_started(
                    "a-exec",
                    "a",
                    NodeKindName::Session,
                    Some(ExecutionParentRef::sequence_child("main-exec")),
                    1.0,
                ),
                WorkflowEvent::NodeFailed {
                    execution_id: TREE.into(),
                    node_execution_id: "a-exec".into(),
                    node_name: "a".into(),
                    attempt: 1,
                    reason: "prepare failed".into(),
                    retry_count: None,
                    failure_kind:
                        crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                    timestamp: 2.0,
                },
            ],
        )
        .await
        .unwrap();
        // When
        let recovered = reconcile_tree_pass(&store, TREE, 3.0, &mut test_id_source())
            .await
            .unwrap()
            .unwrap();
        // Then
        assert!(recovered.starts.is_empty());
        assert_eq!(
            recovered
                .folded
                .aggregate
                .node_execution("a-exec")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Running
        );
        assert!(!read_tree_records(&store, TREE)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
        // Given
        append_facts_for_events(
            &store,
            &[WorkflowEvent::SessionAttached {
                execution_id: TREE.into(),
                node_execution_id: "a-exec".into(),
                session_id: "agent".into(),
                timestamp: 4.0,
            }],
        )
        .await
        .unwrap();
        // When
        let recovered = reconcile_tree_pass(&store, TREE, 5.0, &mut test_id_source())
            .await
            .unwrap()
            .unwrap();
        // Then
        assert!(recovered.starts.is_empty());
        let records = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| matches!(record.fact, NodeFact::ProcessExited(_)))
                .count(),
            0
        );
    }

    async fn row_count(store: &std::sync::Arc<LocalEventStore>) -> usize {
        read_tree_records(store, TREE).await.unwrap().len()
    }

    #[tokio::test]
    async fn test_delegate復旧_提出だけ保存された木のchild開始を追記し再導出しない() {
        use crate::domain::workflow::entities::workflow_execution::{
            ExecutionAdvanceDecision, PendingAdvance,
        };
        // Given
        let (_directory, store) = open_store();
        let definition: WorkflowDefinition = serde_saphyr::from_str("name: delegate\ndescription: test\nnodes:\n  main: {session: {provider: codex}, artifact: result, completion: {delegate: {child: verify, when: child.ok, max_iterations: 2}}}\n  verify: {command: check}\nschemas:\n  result: {type: object, properties: {}, required: []}").unwrap();
        let parent_meta = test_fact_meta(TREE, "parent");
        let root = TreeRootFact {
            repository_root: None,
            workspace_identity: "/repo".into(),
            worktree_path: "/repo".into(),
            created_from: ExecutionOrigin::Cli,
            request: "please".into(),
            workflow_name: definition.name.clone(),
            definition: Some(definition),

            launched_as: ExecutionTreeLaunch::Workflow,
        };
        for (index, fact) in [
            NodeFact::Started(StartedFact {
                worktree: None,
                parent: None,
                root: Some(Box::new(root)),
            }),
            NodeFact::SessionAttached(SessionAttachedFact {
                session_id: "agent".into(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: false,
            }),
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("result".into()),
                value: serde_json::json!({}),
                request_id: None,
            }),
        ]
        .iter()
        .enumerate()
        {
            append_single_fact(&store, &parent_meta, fact, (index as i64 + 1) * 1000)
                .await
                .unwrap();
        }
        let mut folded = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        let advance = PendingAdvance::Delegate {
            node_execution_id: "parent".into(),
        };
        assert_eq!(
            folded.aggregate.derive_pending_advances(),
            [advance.clone()]
        );
        // When
        let applied = folded
            .aggregate
            .apply_pending_advance(&advance, &mut || "child".into(), 5.0)
            .unwrap();
        append_facts_for_events(&store, &applied.events)
            .await
            .unwrap();
        let restored = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        // Then
        let ExecutionAdvanceDecision::StartNodes(starts) = applied.decision else {
            panic!()
        };
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].node_execution_id(), "child");
        assert!(restored.aggregate.derive_pending_advances().is_empty());
        assert_eq!(
            restored.aggregate.node_executions,
            folded.aggregate.node_executions
        );
        assert_eq!(
            read_tree_records(&store, TREE)
                .await
                .unwrap()
                .iter()
                .filter(|record| record.meta.node_name == "verify"
                    && matches!(record.fact, NodeFact::Started(_)))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn test_session起動由来のstop受信済みnodeをreconcileしてもattentionを維持する() {
        let (_root, store) = open_store();
        let session_id = "agent-session-restart";
        LocalAgentSessionRepository::new(store.clone())
            .create(
                AgentSession::create(
                    session_id,
                    WorkspaceIdentity::new("/repo"),
                    "/repo",
                    crate::domain::provider_lifecycle::ProviderKind::Codex,
                    AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                )
                .unwrap(),
                "create-restart-session",
            )
            .await
            .unwrap();
        append_facts_for_events(
            &store,
            &[WorkflowEvent::NodeStopReceived {
                execution_id: session_id.to_string(),
                node_execution_id: session_id.to_string(),
                timestamp: 2.0,
            }],
        )
        .await
        .unwrap();
        assert!(!read_tree_records(&store, session_id)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));

        let mut new_id = test_id_source();
        let reconciliation = reconcile_tree_pass(&store, session_id, 10.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();

        assert!(reconciliation.starts.is_empty());
        assert_eq!(
            reconciliation
                .folded
                .aggregate
                .node_execution(session_id)
                .unwrap()
                .completion_signals,
            crate::domain::workflow::NodeCompletionSignalState::StopReceived
        );
        assert!(!read_tree_records(&store, session_id)
            .await
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
        let node = SqliteWorkspaceTreeRepository::new(store)
            .load_node(&WorkspaceIdentity::new("/repo"), session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            node.status_classification,
            WorkspaceNodeStatusClassification::Attention
        );
    }

    /// ISSUE 受け入れ基準: 任意の時点で kill しても、再起動後の reconciliation が
    /// 未実行の行動を検出して継続し、行動の二重実行が起きない。
    /// kill 点: 子の完了導出（stop 事実）と次の子の started の間。
    #[tokio::test]
    async fn test_再入_完了と次の開始の間でkillされた前進を検出して継続する() {
        let (_root, store) = open_store();
        // Given: a の完了二信号まで（次の run の started が無い = kill 点）
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
                node_started(
                    "a-exec",
                    "a",
                    NodeKindName::Session,
                    Some(ExecutionParentRef::sequence_child("main-exec")),
                    1.0,
                ),
                WorkflowEvent::NodeSubmitReceived {
                    execution_id: TREE.to_string(),
                    node_execution_id: "a-exec".to_string(),
                    timestamp: 2.0,
                },
                WorkflowEvent::NodeStopReceived {
                    execution_id: TREE.to_string(),
                    node_execution_id: "a-exec".to_string(),
                    timestamp: 3.0,
                },
            ],
        )
        .await
        .unwrap();
        let before = row_count(&store).await;

        // When: reconciliation パスを実行する
        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();

        // Then: 次の子（run command）の started が追記され、起動対象として返る
        assert_eq!(outcome.starts.len(), 1);
        assert_eq!(outcome.starts[0].node_name(), "run");
        let records = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(records.len(), before + 1);
        let last = records.last().unwrap();
        assert_eq!(fact_codec::event_type(&last.fact), "started");
        assert_eq!(last.meta.node_name, "run");

        // Then: started だけが永続化された kill 点では同じ leaf を再び起動対象に返し、
        // started を重複して追記しない。
        let mut new_id = test_id_source();
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.starts, outcome.starts);
        let started_rows = |records: &[crate::domain::workflow::NodeFactRecord]| {
            records
                .iter()
                .filter(|record| fact_codec::event_type(&record.fact) == "started")
                .count()
        };
        let after_second = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(after_second.len(), records.len());
        assert_eq!(started_rows(&after_second), started_rows(&records));

        // spawn 済みなら、次回起動でも喪失を追記せず再起動しない。
        append_facts_for_events(
            &store,
            &[WorkflowEvent::CommandSpawned {
                execution_id: TREE.to_string(),
                node_execution_id: outcome.starts[0].node_execution_id().to_string(),
                display_command: "true".to_string(),
                timestamp: 11.5,
            }],
        )
        .await
        .unwrap();
        let mut new_id = test_id_source();
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert!(third.starts.is_empty());
        let after_third = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(
            fact_codec::event_type(&after_third.last().unwrap().fact),
            "command_spawned"
        );

        // 以後の読み取りでも記録は変わらない。
        let count_after_third = after_third.len();
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store).await, count_after_third);
    }

    /// kill 点: 合成子の started と実効 entry の子の started の間。
    #[tokio::test]
    async fn test_再入_entry未開始のsequenceに実効entryの開始を補完する() {
        let (_root, store) = open_store();
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
            ],
        )
        .await
        .unwrap();

        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();

        // Then: entry の子 a が開始される
        assert_eq!(outcome.starts.len(), 1);
        assert_eq!(outcome.starts[0].node_name(), "a");
        let records = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(records.last().unwrap().meta.node_name, "a");

        // provider lifecycle の準備は node_events 上の外部実行成立事実ではない。
        // attach 前に kill された場合、2周目も同じ leaf を返し、started は増やさない。
        let before_second = read_tree_records(&store, TREE).await.unwrap();
        let expected_record_count = before_second.len();
        let started_count = before_second
            .iter()
            .filter(|record| fact_codec::event_type(&record.fact) == "started")
            .count();
        let mut new_id = test_id_source();
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.starts, outcome.starts);
        let after_second = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(after_second.len(), expected_record_count);
        assert_eq!(
            after_second
                .iter()
                .filter(|record| fact_codec::event_type(&record.fact) == "started")
                .count(),
            started_count
        );

        append_facts_for_events(
            &store,
            &[WorkflowEvent::SessionAttached {
                execution_id: TREE.to_string(),
                node_execution_id: outcome.starts[0].node_execution_id().to_string(),
                session_id: "session-1".to_string(),
                timestamp: 11.5,
            }],
        )
        .await
        .unwrap();
        let mut new_id = test_id_source();
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert!(third.starts.is_empty());
        let after_third = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(
            fact_codec::event_type(&after_third.last().unwrap().fact),
            "session_attached"
        );

        let count_after_third = after_third.len();
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store).await, count_after_third);
    }

    /// kill 点: 実行中プロセスごと落ちた場合。記録と Running を維持する。
    #[tokio::test]
    async fn test_再入_実行中プロセスの喪失を追記せずrunningを維持する() {
        let (_root, store) = open_store();
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
                node_started(
                    "a-exec",
                    "a",
                    NodeKindName::Session,
                    Some(ExecutionParentRef::sequence_child("main-exec")),
                    1.0,
                ),
                WorkflowEvent::SessionAttached {
                    execution_id: TREE.to_string(),
                    node_execution_id: "a-exec".to_string(),
                    session_id: "session-1".to_string(),
                    timestamp: 2.0,
                },
            ],
        )
        .await
        .unwrap();

        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();

        // Then: 起動時にはプロセス喪失を記録せず、node と木は Running
        assert!(outcome.starts.is_empty());
        let records = read_tree_records(&store, TREE).await.unwrap();
        assert_eq!(
            fact_codec::event_type(&records.last().unwrap().fact),
            "session_attached"
        );
        assert_eq!(
            outcome
                .folded
                .aggregate
                .node_execution("a-exec")
                .map(|node| node.status),
            Some(RuntimeNodeExecutionStatus::Running)
        );
        assert_eq!(
            *outcome.folded.aggregate.state(),
            RuntimeExecutionState::Running
        );

        // 冪等: 2周目にも追記しない
        let count = row_count(&store).await;
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store).await, count);
    }
}

mod round_trip_tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
    #[tokio::test]
    async fn test_session起動由来seedはrootとattachmentを同じdurable_batchで記録する() {
        // Given: Session 起動由来の木を構成する root と attachment
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let session_id = "agent-session-seed-atomic";
        let facts = SessionExecutionTreeRootFacts::new(
            session_id,
            "workspace-1",
            "/repo/.worktrees/feature",
            crate::domain::provider_lifecycle::ProviderKind::Codex,
            None,
        )
        .unwrap()
        .into_facts();
        store.fault_injector().arm_fail_after_participant_write(1);

        // When: root 書き込み直後に batch を失敗させる
        let failed = append_fact_batch_for_seed(&store, &facts, 1, "session-seed-atomic");

        // Then: root だけの中間状態は durable にならず、同じ batch を再試行できる
        assert!(failed.is_err());
        assert!(read_tree_records(&store, session_id)
            .await
            .unwrap()
            .is_empty());
        append_fact_batch_for_seed(&store, &facts, 1, "session-seed-atomic").unwrap();
        let records = read_tree_records(&store, session_id).await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(fact_codec::event_type(&records[0].fact), "started");
        assert_eq!(fact_codec::event_type(&records[1].fact), "session_attached");
    }

    /// エンジンが発するイベント列を写像して append した事実ログが、
    /// fold で同じ実行木として導出されることの統合確認。
    #[tokio::test]
    async fn test_store経由_写像した事実ログをfoldすると実行木が導出される() {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();

        // Given: 起動 → a(session) 完了 → run(command) 完了 の live イベント列
        let batches: Vec<Vec<WorkflowEvent>> = vec![
            vec![
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
                node_started(
                    "a-exec",
                    "a",
                    NodeKindName::Session,
                    Some(ExecutionParentRef::sequence_child("main-exec")),
                    1.0,
                ),
            ],
            vec![WorkflowEvent::SessionAttached {
                execution_id: TREE.to_string(),
                node_execution_id: "a-exec".to_string(),
                session_id: "session-1".to_string(),
                timestamp: 2.0,
            }],
            vec![WorkflowEvent::NodeSubmitReceived {
                execution_id: TREE.to_string(),
                node_execution_id: "a-exec".to_string(),
                timestamp: 3.0,
            }],
            vec![
                WorkflowEvent::NodeStopReceived {
                    execution_id: TREE.to_string(),
                    node_execution_id: "a-exec".to_string(),
                    timestamp: 4.0,
                },
                // 完了と次 node の開始（engine の advance が出す形）
                WorkflowEvent::NodeCompleted {
                    execution_id: TREE.to_string(),
                    node_execution_id: "a-exec".to_string(),
                    node_name: "a".to_string(),
                    attempt: 1,
                    result_summary: None,
                    token_usage: None,
                    timestamp: 4.0,
                },
                node_started(
                    "run-exec",
                    "run",
                    NodeKindName::Command,
                    Some(ExecutionParentRef::sequence_child("main-exec")),
                    4.0,
                ),
            ],
            vec![WorkflowEvent::CommandSpawned {
                execution_id: TREE.to_string(),
                node_execution_id: "run-exec".to_string(),
                display_command: "true".to_string(),
                timestamp: 5.0,
            }],
            vec![
                WorkflowEvent::NodeCompleted {
                    execution_id: TREE.to_string(),
                    node_execution_id: "run-exec".to_string(),
                    node_name: "run".to_string(),
                    attempt: 1,
                    result_summary: Some("ok".to_string()),
                    token_usage: None,
                    timestamp: 6.0,
                },
                WorkflowEvent::ExecutionCompleted {
                    execution_id: TREE.to_string(),
                    total_token_usage: Default::default(),
                    timestamp: 6.0,
                },
            ],
        ];

        // When: バッチごとに写像して append し、fold する
        for batch in &batches {
            append_facts_for_events(&store, batch).await.unwrap();
        }
        let records = read_tree_records(&store, TREE).await.unwrap();
        let tree = fold_execution_tree(TREE, &records).unwrap().unwrap();

        // Then: 遷移イベントなしの事実ログから同じ完了状態が導出される
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            tree.aggregate
                .node_execution("a-exec")
                .map(|node| node.status),
            Some(RuntimeNodeExecutionStatus::Succeeded)
        );
        assert_eq!(
            tree.aggregate
                .node_execution("run-exec")
                .map(|node| node.status),
            Some(RuntimeNodeExecutionStatus::Succeeded)
        );
        assert!(tree.root.definition.is_some());
        let NodeFact::Started(started) = fact_codec::decode(
            "started",
            &super::append_contract_tests::read_raw_rows(&store, TREE).await[0].detail,
        )
        .unwrap() else {
            panic!()
        };
        let root = started.root.unwrap();
        assert_eq!(
            root.definition
                .as_ref()
                .unwrap()
                .node_by_name("run")
                .and_then(NodeDefinition::command_spec)
                .and_then(|command| {
                    command
                        .env
                        .get(&crate::domain::workflow::EnvironmentVariableName::new("DOC").unwrap())
                })
                .map(crate::domain::workflow::InputParameterRef::as_string),
            Some("document".to_string())
        );
        assert_eq!(
            tree.aggregate
                .node_execution("run-exec")
                .and_then(|node| node.display_command.clone()),
            Some("true".to_string())
        );

        // Then: ログの event_type はすべて純粋事実の語彙
        for record in &records {
            assert!(matches!(
                fact_codec::event_type(&record.fact),
                "started"
                    | "session_attached"
                    | "command_spawned"
                    | "process_exited"
                    | "submit_received"
                    | "submit_rejected"
                    | "stop_received"
                    | "artifact_produced"
                    | "approval_granted"
                    | "retry_requested"
                    | "resume_requested"
                    | "execution_completed"
                    | "abort_requested"
                    | "archive_requested"
                    | "restore_requested"
            ));
        }
    }
}

#[tokio::test]
async fn test_旧隔離事実の読取_状態導出と再起動復元からだけ除外する() {
    use crate::domain::local_event::{
        CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryResult,
        LocalEventTransactionRepository,
    };
    use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;
    // Given
    let directory = tempfile::TempDir::new().unwrap();
    let config = LocalEventStoreConfig::production(directory.path().to_path_buf());
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    append_facts_for_events(
        &store,
        &[
            started_event(),
            node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
            node_started(
                "a-exec",
                "a",
                NodeKindName::Session,
                Some(ExecutionParentRef::sequence_child("main-exec")),
                2.0,
            ),
        ],
    )
    .await
    .unwrap();
    append_single_fact(
        &store,
        &NodeFactMeta {
            tree_id: TREE.into(),
            node_execution_id: "a-exec".into(),
            parent_id: Some("main-exec".into()),
            node_name: "a".into(),
            kind: NodeKindName::Session,
            attempt: 1,
        },
        &NodeFact::SessionAttached(SessionAttachedFact {
            session_id: "legacy-session".into(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: true,
        }),
        2000,
    )
    .await
    .unwrap();
    let original = read_tree_records(&store, TREE).await.unwrap();
    let rows = [
        (
            "isolated_worktree_created",
            serde_json::json!({
                "repositoryRoot": "/repo", "worktreePath": "/old-isolated", "branch": "old-branch"
            }),
        ),
        ("isolated_worktree_released", serde_json::json!({})),
        ("isolated_worktree_lost", serde_json::json!({})),
    ]
    .into_iter()
    .map(|(event_type, detail)| PendingFactRow {
        row: NewNodeEventRow {
            tree_id: TREE.into(),
            node_execution_id: "a-exec".into(),
            parent_id: Some("main-exec".into()),
            node_name: "a".into(),
            kind: "session".into(),
            attempt: 1,
            event_type: event_type.into(),
            session_id: None,
            detail: detail.to_string(),
        },
        timestamp_ms: 3000,
    })
    .collect();
    append_pending_rows(&store, rows).await.unwrap();
    drop(store);

    // When
    let store = LocalEventStore::open(config).unwrap();
    let readonly =
        crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
            directory.path(),
        )
        .unwrap();
    for backend in [
        FactLogReadBackend::Live(store.clone()),
        FactLogReadBackend::ReadOnly(readonly),
    ] {
        let records = read_tree_records_from(&backend, TREE).await.unwrap();
        // Then
        assert_eq!(records, original);
        let tree = fold_tree_from(&backend, TREE).await.unwrap().unwrap();
        let node = tree.aggregate.node_execution("a-exec").unwrap();
        assert!(node.worktree.is_none());
        assert_eq!(tree.aggregate.state(), &RuntimeExecutionState::Running);
    }
    use crate::domain::agent_session::repository::AgentSessionRepository;
    let session =
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone())
            .find("legacy-session")
            .await
            .unwrap();
    assert!(session.is_some());
    let status = super::super::execution_projection_repository::WorkflowExecutionProjectionLogRepository::new(store.clone())
        .get_execution(&crate::domain::workflow::ExecutionTreeId::new(TREE).unwrap()).await.unwrap().unwrap();
    assert_eq!(
        status.status,
        crate::domain::workflow::ExecutionStatus::Running
    );
    let owners = store
        .query(LocalEventQuery::CanonicalRuntimeOwnerSnapshot { limit: 10 })
        .await
        .unwrap();
    assert!(
        matches!(owners, LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(owners)
        if owners == vec![CanonicalRuntimeOwnerView::ActiveWorkflow { worktree_path: "/repo".into() }])
    );
    let restored = reconcile_tree_pass(&store, TREE, 5.0, &mut || "next-exec".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.folded.aggregate.state(),
        &RuntimeExecutionState::Running
    );
    let rows = FactLogReadBackend::Live(store)
        .run_indexed(|connection| {
            node_events::read_tree(connection, TREE)
                .map_err(|_| LocalEventQueryError::InvalidRequest)
        })
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| row.event_type.starts_with("isolated_worktree_"))
            .count(),
        3
    );
}

#[test]
fn test_終端復元_旧worktreeは同じnodeのattemptの欠落だけを補い終端後を無視する() {
    // Given
    let legacy = crate::domain::workflow::IsolatedWorktree {
        branch: "old-branch".into(),
        path: "/old-isolated".into(),
    };
    let current = crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "node", 1);
    for (node_id, attempt, after_terminal, recorded, expected) in [
        ("node", 1, false, None, Some(legacy.clone())),
        ("other", 1, false, None, None),
        ("node", 2, false, None, None),
        ("node", 1, true, None, None),
        ("node", 1, false, Some(current.clone()), Some(current)),
    ] {
        let started = NodeEventRow {
            tree_id: TREE.into(),
            seq: 1,
            node_execution_id: "node".into(),
            parent_id: None,
            node_name: "main".into(),
            kind: "session".into(),
            attempt: 1,
            event_type: "started".into(),
            session_id: None,
            detail: fact_codec::encode_detail(&NodeFact::Started(StartedFact {
                worktree: recorded,
                parent: None,
                root: None,
            }))
            .unwrap(),
            timestamp_ms: 1_000,
        };
        let worktree = NodeEventRow {
            seq: if after_terminal { 4 } else { 2 },
            node_execution_id: node_id.into(),
            attempt,
            event_type: "isolated_worktree_created".into(),
            detail: serde_json::json!({
                "repositoryRoot": "/repo", "worktreePath": legacy.path, "branch": legacy.branch,
            })
            .to_string(),
            ..started.clone()
        };
        let terminal = NodeEventRow {
            seq: 3,
            event_type: "abort_requested".into(),
            detail: "{}".into(),
            ..started.clone()
        };
        let rows = if after_terminal {
            vec![started, terminal, worktree]
        } else {
            vec![started, worktree, terminal]
        };

        // When
        let records = records_from_tree_rows(&rows).unwrap();

        // Then
        let NodeFact::Started(started) = &records[0].fact else {
            panic!("started")
        };
        assert_eq!(started.worktree, expected);
        assert_eq!(records.len(), 2);
    }
}

#[test]
fn test_旧隔離事実の読取_破損payloadと未知の事実を拒否する() {
    // Given
    for (event_type, detail) in [
        ("isolated_worktree_created", "{}"),
        (
            "isolated_worktree_created",
            r#"{"repositoryRoot":1,"worktreePath":"/tmp","branch":"b"}"#,
        ),
        ("isolated_worktree_released", "null"),
        ("isolated_worktree_lost", "[]"),
        ("isolated_worktree_unknown", "{}"),
    ] {
        // When / Then
        assert!(
            decode_stored_fact(event_type, detail, 0).is_err(),
            "{event_type}: {detail}"
        );
    }
}

#[test]
fn test_終端復元_定義本文を入れ替えても名前と実行状態と公開属性が変わらない() {
    // Given
    let started = NodeEventRow {
        tree_id: TREE.into(),
        seq: 1,
        node_execution_id: "node".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: "command".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: None,
        detail: String::new(),
        timestamp_ms: 1_000,
    };
    for event_type in ["execution_completed", "abort_requested"] {
        let terminal = NodeEventRow {
            seq: 2,
            event_type: event_type.into(),
            detail: "{}".into(),
            timestamp_ms: 2_000,
            ..started.clone()
        };
        let mut expected = None;
        for definition in [
            serde_json::json!({"name": "different-name", "entry": "main", "nodes": {"main": {"command": "true"}}}),
            serde_json::json!({"name": 42, "nodes": {"main": {"completion": "approval"}}}),
            serde_json::Value::Null,
        ] {
            let mut started = started.clone();
            started.detail = serde_json::json!({"root": {
                "repositoryRoot": "/repo", "workspaceIdentity": "/repo", "worktreePath": "/repo",
                "createdFrom": "cli", "request": "please", "launchedAs": "workflow",
                "workflowName": "recorded-name", "definition": definition
            }})
            .to_string();
            // When
            let records = records_from_tree_rows(&[started, terminal.clone()]).unwrap();
            let tree = fold_execution_tree(TREE, &records).unwrap().unwrap();
            let model = crate::domain::workflow::services::fact_replay::derive_read_model(&tree);
            // Then
            assert!(tree.root.definition.is_none());
            assert!(tree.aggregate.workflow.is_none());
            assert_eq!(model.workflow_name, "recorded-name");
            assert_eq!(model.completed_at, Some(2.0));
            assert_eq!(model.node_executions.len(), 1);
            if let Some(expected) = &expected {
                assert_eq!(&model, expected);
            } else {
                expected = Some(model);
            }
        }
    }
}

mod terminal_fact_tests {
    use super::append_contract_tests::read_raw_rows;
    use super::*;
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    use crate::domain::workflow::ExecutionStatus;

    #[tokio::test]
    async fn test_完了記録_終端行の保存失敗で完了信号も巻き戻り再試行で両方が残る() {
        use crate::adaptor::gateway::local_event_store::layout::StoreLayout;

        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let mut started = started_event();
        if let WorkflowEvent::ExecutionStarted { definition, .. } = &mut started {
            *definition = serde_saphyr::from_str(
                "name: command\ndescription: test\nnodes:\n  main: {command: 'true'}\n",
            )
            .unwrap();
        }
        append_facts_for_events(
            &store,
            &[
                started,
                node_started("root", "main", NodeKindName::Command, None, 1.0),
            ],
        )
        .await
        .unwrap();
        let mut folded = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        let completed = folded
            .aggregate
            .complete_leaf_and_advance("root", &mut || panic!("must not start"), 2.0)
            .unwrap();
        let before = read_raw_rows(&store, TREE).await;
        let connection =
            rusqlite::Connection::open(StoreLayout::new(dir.path()).database_path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_completion BEFORE INSERT ON node_events
                 WHEN NEW.event_type = 'execution_completed'
                 BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END;",
            )
            .unwrap();

        // When
        assert!(append_facts_for_events(&store, &completed.events)
            .await
            .is_err());
        assert_eq!(read_raw_rows(&store, TREE).await, before);
        drop(store);
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();

        // Then
        let folded = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        assert!(folded.aggregate.is_active());
        connection
            .execute_batch("DROP TRIGGER fail_completion;")
            .unwrap();
        append_facts_for_events(&store, &completed.events)
            .await
            .unwrap();
        drop(store);
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let rows = read_raw_rows(&store, TREE).await;
        assert_eq!(rows.len(), before.len() + 2);
        assert_eq!(rows[before.len()].event_type, "process_exited");
        assert_eq!(rows[before.len() + 1].event_type, "execution_completed");
        assert_eq!(rows[before.len()].timestamp_ms, 2_000);
        assert_eq!(rows[before.len() + 1].timestamp_ms, 2_000);
        let recovered = reconcile_tree_pass(&store, TREE, 3.0, &mut || panic!("must not start"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered.folded.aggregate.state(),
            &RuntimeExecutionState::Completed
        );
        assert_eq!(read_raw_rows(&store, TREE).await, rows);
    }

    #[tokio::test]
    async fn test_完了seed_完了事実を保存して再seedでも重複しない() {
        use crate::adaptor::gateway::workflow::test_support::seed_canonical_execution;
        use crate::domain::workflow::WorkflowExecutionSummary as WorkflowExecutionMetadata;

        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let execution = WorkflowExecutionMetadata {
            execution_id: TREE.into(),
            workflow_name: "completed-seed".into(),
            status: ExecutionStatus::Completed,
            worktree_path: "/repo".into(),
            current_node: None,
            created_from: ExecutionOrigin::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: Some(2.0),
            error_reason: None,
            total_token_usage: Default::default(),
        };

        // When
        seed_canonical_execution(&store, &execution, &[]).await;
        seed_canonical_execution(&store, &execution, &[]).await;

        // Then
        let rows = read_raw_rows(&store, TREE).await;
        assert_eq!(
            rows.iter()
                .map(|row| row.event_type.as_str())
                .collect::<Vec<_>>(),
            [
                "started",
                "submit_received",
                "stop_received",
                "execution_completed"
            ]
        );
        assert_eq!(rows.last().unwrap().timestamp_ms, 2_000);
        let folded = fold_tree_from(&FactLogReadBackend::Live(store), TREE)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
    }

    #[tokio::test]
    async fn test_終端復元_保存定義の承認待ちnodeをwriterとreadonlyで同じに復元する() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let mut started = started_event();
        if let WorkflowEvent::ExecutionStarted { definition, .. } = &mut started {
            *definition = serde_saphyr::from_str(
                "name: wf\ndescription: test\nnodes:\n  main:\n    session: {provider: codex}\n    completion: {require: approval}\n",
            ).unwrap();
        }
        append_facts_for_events(
            &store,
            &[
                started,
                node_started("root", "main", NodeKindName::Session, None, 1.0),
            ],
        )
        .await
        .unwrap();
        let root = read_tree_records(&store, TREE).await.unwrap()[0]
            .meta
            .clone();
        for (timestamp, fact) in [
            (
                2_000,
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            ),
            (
                3_000,
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: Some("done".into()),
                    token_usage: None,
                }),
            ),
        ] {
            append_single_fact(&store, &root, &fact, timestamp)
                .await
                .unwrap();
        }
        let mut expected = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            expected.aggregate.node_execution("root").unwrap().status,
            crate::domain::workflow::NodeExecutionStatus::WaitingApproval
        );
        expected.aggregate.replay_aborted_at(4.0, None);
        let expected = crate::domain::workflow::services::fact_replay::derive_read_model(&expected);
        append_single_fact(
            &store,
            &root,
            &NodeFact::AbortRequested(Default::default()),
            4_000,
        )
        .await
        .unwrap();
        let before = read_raw_rows(&store, TREE).await;
        let readonly =
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                dir.path(),
            )
            .unwrap();

        // When / Then
        for backend in [
            FactLogReadBackend::Live(store.clone()),
            FactLogReadBackend::ReadOnly(readonly),
        ] {
            let tree = fold_tree_from(&backend, TREE).await.unwrap().unwrap();
            assert!(tree.root.definition.is_some());
            assert_eq!(
                crate::domain::workflow::services::fact_replay::derive_read_model(&tree),
                expected
            );
        }
        assert_eq!(read_raw_rows(&store, TREE).await, before);
    }

    async fn legacy_tree(store: &Arc<LocalEventStore>, completed_signals: bool) -> NodeFactMeta {
        let meta = NodeFactMeta {
            tree_id: TREE.into(),
            node_execution_id: TREE.into(),
            parent_id: None,
            node_name: "main".into(),
            kind: NodeKindName::Command,
            attempt: 1,
        };
        store
            .append_node_event(
                NewNodeEventRow {
                    tree_id: TREE.into(),
                    node_execution_id: TREE.into(),
                    parent_id: None,
                    node_name: "main".into(),
                    kind: "command".into(),
                    attempt: 1,
                    event_type: "started".into(),
                    session_id: None,
                    detail: serde_json::json!({"root": {
                        "workspaceIdentity": "/repo", "worktreePath": "/repo", "createdFrom": "cli",
                        "request": "", "launchedAs": "workflow",
                        "definition": {"name": "old", "description": "", "entry": "main", "nodes": {
                            "main": {"command": "true", "completion": "auto"}
                        }}
                    }})
                    .to_string(),
                },
                Some(1_000),
            )
            .await
            .unwrap();
        if completed_signals {
            append_single_fact(
                store,
                &meta,
                &NodeFact::ProcessExited(ProcessExitedFact {
                    exit_code: Some(0),
                    result_summary: Some("done".into()),
                    failure_reason: None,
                    failure_kind: None,
                }),
                2_000,
            )
            .await
            .unwrap();
        }
        meta
    }

    #[tokio::test]
    async fn test_起動時定義確認_書込が失敗する状態でもabortを保存しない() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        legacy_tree(&store, true).await;
        let connection =
            rusqlite::Connection::open(StoreLayout::new(dir.path()).database_path()).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_startup_abort BEFORE INSERT ON node_events WHEN NEW.event_type = 'abort_requested' BEGIN SELECT RAISE(ABORT, 'injected abort failure'); END;").unwrap();
        let repository =
            crate::adaptor::gateway::workflow::startup_repository::StoredWorkflowStartupRepository(
                store.clone(),
            );
        let before = read_raw_rows(&store, TREE).await;
        for _ in 0..2 {
            assert!(matches!(
                crate::usecase::workflow::startup::check_startup_definition(&repository, TREE).await,
                Err(crate::domain::workflow::WorkflowError::IncompatibleStoredEvent(reason)) if reason.contains("completion")
            ));
            assert_eq!(read_raw_rows(&store, TREE).await, before);
        }
    }

    #[tokio::test]
    async fn test_起動時定義確認_旧定義の未完了と完了事実のない過去完了をabortしない() {
        for completed_signals in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into()))
                .unwrap();
            legacy_tree(&store, completed_signals).await;
            let backend = FactLogReadBackend::Live(store.clone());
            let before = read_raw_rows(&store, TREE).await;
            let repository = crate::adaptor::gateway::workflow::startup_repository::StoredWorkflowStartupRepository(store.clone());
            for _ in 0..2 {
                assert!(matches!(
                    crate::usecase::workflow::startup::check_startup_definition(&repository, TREE)
                        .await,
                    Err(crate::domain::workflow::WorkflowError::IncompatibleStoredEvent(_))
                ));
                assert!(fold_tree_from(&backend, TREE).await.is_err());
                assert_eq!(read_raw_rows(&store, TREE).await, before);
            }
        }
    }

    #[tokio::test]
    async fn test_終端復元_旧隔離worktreeをabort済みnodeと提出artifactに保持する() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let meta = legacy_tree(&store, false).await;
        for (event_type, detail) in [
            (
                "isolated_worktree_created",
                serde_json::json!({
                    "repositoryRoot": "/repo", "worktreePath": "/old-isolated", "branch": "old-branch"
                }),
            ),
            ("isolated_worktree_released", serde_json::json!({})),
            ("isolated_worktree_lost", serde_json::json!({})),
        ] {
            store
                .append_node_event(
                    NewNodeEventRow {
                        tree_id: TREE.into(),
                        node_execution_id: TREE.into(),
                        parent_id: None,
                        node_name: "main".into(),
                        kind: "command".into(),
                        attempt: 1,
                        event_type: event_type.into(),
                        session_id: None,
                        detail: detail.to_string(),
                    },
                    Some(2_000),
                )
                .await
                .unwrap();
        }
        append_single_fact(
            &store,
            &meta,
            &NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: Some("result".into()),
                value: serde_json::json!({"ok": true}),
                request_id: None,
            }),
            3_000,
        )
        .await
        .unwrap();
        append_single_fact(
            &store,
            &meta,
            &NodeFact::AbortRequested(Default::default()),
            4_000,
        )
        .await
        .unwrap();
        let before = read_raw_rows(&store, TREE).await;
        drop(store);

        // When
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let readonly =
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                dir.path(),
            )
            .unwrap();
        for backend in [
            FactLogReadBackend::Live(store.clone()),
            FactLogReadBackend::ReadOnly(readonly),
        ] {
            let tree = fold_tree_from(&backend, TREE).await.unwrap().unwrap();
            let model = crate::domain::workflow::services::fact_replay::derive_read_model(&tree);

            // Then
            assert_eq!(model.status, ExecutionStatus::Aborted);
            let node = &model.node_executions[0];
            assert_eq!(
                node.status,
                crate::domain::workflow::NodeExecutionStatus::Aborted
            );
            assert_eq!(
                node.worktree,
                Some(crate::domain::workflow::IsolatedWorktree {
                    path: "/old-isolated".into(),
                    branch: "old-branch".into(),
                })
            );
            assert_eq!(
                node.artifact.as_ref().unwrap().value,
                serde_json::json!({
                    "ok": true, "worktree": {"path": "/old-isolated", "branch": "old-branch"}
                })
            );
        }
        assert_eq!(read_raw_rows(&store, TREE).await, before);
    }

    #[tokio::test]
    async fn test_終端復元_旧定義でも完了とabortを保持し起動時に事実を追加しない() {
        // Given
        for terminal in [
            NodeFact::ExecutionCompleted,
            NodeFact::AbortRequested(Default::default()),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into()))
                .unwrap();
            let meta = legacy_tree(&store, true).await;
            append_single_fact(&store, &meta, &terminal, 3_000)
                .await
                .unwrap();
            let before = read_raw_rows(&store, TREE).await.len();
            let readonly =
                crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                    dir.path(),
                )
                .unwrap();
            // When / Then
            for backend in [
                FactLogReadBackend::Live(store.clone()),
                FactLogReadBackend::ReadOnly(readonly),
            ] {
                let folded = fold_tree_from(&backend, TREE).await.unwrap().unwrap();
                assert_eq!(
                    folded.aggregate.state(),
                    &terminal.terminal_state().unwrap()
                );
                assert_eq!(folded.aggregate.updated_at, 3.0);
            }
            let recovered =
                reconcile_tree_pass(&store, TREE, 4.0, &mut || panic!("must not start"))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                recovered.folded.aggregate.state(),
                &terminal.terminal_state().unwrap()
            );
            assert_eq!(read_raw_rows(&store, TREE).await.len(), before);
        }
    }

    #[tokio::test]
    async fn test_終端復元_現行形式の定義でも再生規則が一致しなければ完了事実を優先する() {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
        let mut events = vec![
            started_event(),
            node_started("root", "renamed", NodeKindName::Command, None, 1.0),
        ];
        events.push(WorkflowEvent::ExecutionCompleted {
            execution_id: TREE.into(),
            total_token_usage: Default::default(),
            timestamp: 2.0,
        });
        // When
        append_facts_for_events(&store, &events).await.unwrap();
        let folded = fold_tree_from(&FactLogReadBackend::Live(store.clone()), TREE)
            .await
            .unwrap()
            .unwrap();
        // Then
        assert_eq!(folded.aggregate.state(), &RuntimeExecutionState::Completed);
        assert_eq!(
            read_raw_rows(&store, TREE).await.last().unwrap().event_type,
            "execution_completed"
        );
        assert_eq!(
            folded.aggregate.node_execution("root").unwrap().status,
            crate::domain::workflow::NodeExecutionStatus::Succeeded
        );
    }
}

#[tokio::test]
async fn test_起動時前進_head競合を失敗と区別し最新記録から再評価できる() {
    for (abort, drop_reply) in [(false, false), (true, false), (false, true), (true, true)] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
            ],
        )
        .await
        .unwrap();
        let meta = read_tree_records(&store, TREE).await.unwrap()[0]
            .meta
            .clone();
        let mut injected = false;
        // When
        let result = reconcile_tree_pass(&store, TREE, 2.0, &mut || {
            assert!(!injected);
            injected = true;
            let runtime = tokio::runtime::Handle::current();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        runtime.block_on(async {
                            append_single_fact(
                                &store,
                                &meta,
                                &if abort {
                                    NodeFact::AbortRequested(Default::default())
                                } else {
                                    NodeFact::ResumeRequested
                                },
                                2000,
                            )
                            .await
                            .unwrap();
                        })
                    })
                    .join()
                    .unwrap();
            });
            if drop_reply {
                store.fault_injector().arm_drop_reply();
            }
            "conflicting-child".into()
        })
        .await;
        // Then
        assert!(matches!(
            result,
            Err(crate::domain::workflow::WorkflowError::Conflict(_))
        ));
        assert!(!read_tree_records(&store, TREE)
            .await
            .unwrap()
            .iter()
            .any(|record| record.meta.node_execution_id == "conflicting-child"));
        let recovered = reconcile_tree_pass(&store, TREE, 3.0, &mut || "fresh-child".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.starts.len(), usize::from(!abort));
        assert_eq!(recovered.folded.aggregate.is_active(), !abort);
    }
}

#[tokio::test]
async fn test_起動時前進_旧形式の末尾行を含むheadで追記と応答喪失の読戻しを行う() {
    for event_type in [
        "isolated_worktree_created",
        "isolated_worktree_released",
        "isolated_worktree_lost",
    ] {
        for drop_reply in [false, true] {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let store =
                LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                    .unwrap();
            append_facts_for_events(
                &store,
                &[
                    started_event(),
                    node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
                ],
            )
            .await
            .unwrap();
            append_pending_rows(&store, vec![PendingFactRow {
                row: NewNodeEventRow {
                    tree_id: TREE.into(), node_execution_id: "main-exec".into(),
                    parent_id: None, node_name: "main".into(), kind: "sequence".into(), attempt: 1,
                    event_type: event_type.into(), session_id: None,
                    detail: serde_json::json!({"repositoryRoot": "/repo", "worktreePath": "/old", "branch": "old"}).to_string(),
                }, timestamp_ms: 1500,
            }]).await.unwrap();
            if drop_reply {
                store.fault_injector().arm_drop_reply();
            }
            // When
            let result = reconcile_tree_pass(&store, TREE, 2.0, &mut || "next-child".into())
                .await
                .unwrap()
                .unwrap();
            // Then
            assert_eq!(result.starts.len(), 1);
            assert_eq!(result.starts[0].node_execution_id(), "next-child");
            let records = read_tree_records(&store, TREE).await.unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|row| row.meta.node_execution_id == "next-child")
                    .count(),
                1
            );
            assert!(!records
                .iter()
                .any(|row| matches!(row.fact, NodeFact::AbortRequested(_))));
        }
    }
}

#[tokio::test]
async fn test_追記結果確認_全行一致と競合と未保存を共通の判定で区別する() {
    use crate::domain::local_event::CommitBatchError;
    // Given
    let dir = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(dir.path().into())).unwrap();
    let root = SessionExecutionTreeRootFacts::new(
        "tree",
        "/repo",
        "/repo",
        crate::domain::provider_lifecycle::ProviderKind::Codex,
        None,
    )
    .unwrap();
    let first = pending_single_fact(&root.meta, &root.started, -1).unwrap();
    let second = pending_single_fact(&root.meta, &NodeFact::ExecutionCompleted, 2000).unwrap();
    let rows = vec![first, second];
    // When / Then
    assert_eq!(
        resolve_unknown_append(&store, rows.clone(), Some(0))
            .await
            .unwrap(),
        Err(CommitBatchError::AppendOutcomeUnknown)
    );
    append_pending_rows(&store, rows.clone()).await.unwrap();
    assert_eq!(
        resolve_unknown_append(&store, rows.clone(), Some(0))
            .await
            .unwrap(),
        Ok(vec![1, 2])
    );
    for field in 0..10 {
        let mut changed = rows.clone();
        let pending = &mut changed[1];
        match field {
            0 => pending.row.tree_id.push_str("other"),
            1 => pending.row.node_execution_id.push_str("other"),
            2 => pending.row.parent_id = Some("parent".into()),
            3 => pending.row.node_name.push_str("other"),
            4 => pending.row.kind = "command".into(),
            5 => pending.row.attempt += 1,
            6 => pending.row.event_type = "stop_received".into(),
            7 => pending.row.session_id = Some("session".into()),
            8 => pending.row.detail.push(' '),
            9 => pending.timestamp_ms += 1,
            _ => unreachable!(),
        }
        assert!(
            resolve_unknown_append(&store, changed, Some(0))
                .await
                .unwrap()
                .is_err(),
            "field {field}"
        );
    }
    let mut partial = rows;
    partial.push(partial[1].clone());
    assert_eq!(
        resolve_unknown_append(&store, partial, Some(0))
            .await
            .unwrap(),
        Err(CommitBatchError::TreeHeadConflict)
    );
}

#[test]
fn test_fact読み出し_呼び出し境界で混雑と期限切れと破損の分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode as F;
    // Given
    for (source, expected) in [
        (LocalEventQueryError::QueryBusy, F::Unavailable),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            F::DeadlineExceeded,
        ),
        (
            LocalEventQueryError::Corrupt {
                correlation_id: "id".into(),
            },
            F::DataLoss,
        ),
        (
            LocalEventQueryError::Internal {
                correlation_id: "id".into(),
            },
            F::Internal,
        ),
    ] {
        // When
        let workspace = LocalEventQueryError::from(FactReadError::Query(source.clone()));
        let archive =
            crate::domain::workflow::WorkflowError::from(FactReadError::Query(source.clone()));
        // Then
        assert_eq!(workspace, source);
        assert_eq!(workspace.connect_code(), expected);
        assert_eq!(archive.connect_code(), expected);
    }
    for message in ["invalid fact", "missing session attachment"] {
        assert_eq!(
            LocalEventQueryError::from(FactReadError::Corrupt(message.into())).connect_code(),
            F::DataLoss
        );
        assert_eq!(
            crate::domain::workflow::WorkflowError::from(FactReadError::Corrupt(message.into()))
                .connect_code(),
            F::DataLoss
        );
    }
}

#[tokio::test]
async fn test_fact読み出し_liveとread_onlyでsql失敗の分類をconnectまで保持する() {
    use crate::adaptor::gateway::local_event_store::reader::storage_unavailable;
    use crate::adaptor::presenter::connect::classified_error;
    use connectrpc::ErrorCode;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let live =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let read_only = LocalEventReadStore::open(directory.path()).unwrap();
    for backend in [
        FactLogReadBackend::Live(live),
        FactLogReadBackend::ReadOnly(read_only),
    ] {
        for (code, expected) in [
            (rusqlite::ffi::SQLITE_BUSY, ErrorCode::Unavailable),
            (rusqlite::ffi::SQLITE_LOCKED, ErrorCode::Unavailable),
            (rusqlite::ffi::SQLITE_IOERR, ErrorCode::FailedPrecondition),
            (rusqlite::ffi::SQLITE_CORRUPT, ErrorCode::DataLoss),
            (rusqlite::ffi::SQLITE_NOTADB, ErrorCode::DataLoss),
        ] {
            // When
            let error = backend
                .run_indexed::<(), _>(move |_| {
                    Err(storage_unavailable(&rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(code),
                        None,
                    )))
                })
                .await
                .unwrap_err();
            let fact = FactReadError::Query(error);
            let workflow = crate::domain::workflow::WorkflowError::from(fact);
            // Then
            assert_eq!(classified_error(workflow).code, expected);
        }
    }
}

#[tokio::test]
async fn test_reconciliation読取_復元不能な事実列はdata_lossになる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    append_single_fact(
        &store,
        &test_fact_meta(TREE, "root"),
        &NodeFact::ExecutionCompleted,
        1000,
    )
    .await
    .unwrap();

    // When
    let result = reconcile_tree_pass(&store, TREE, 2.0, &mut || panic!("must not start")).await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("invalid stored facts must fail reconciliation"),
    };

    // Then
    assert!(
        matches!(&error, crate::domain::workflow::WorkflowError::CorruptStoredState(message) if message.contains("does not begin with a started fact"))
    );
    assert_eq!(
        crate::adaptor::presenter::connect::classified_error(error).code,
        connectrpc::ErrorCode::DataLoss
    );
}
