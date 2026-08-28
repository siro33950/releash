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
                    output: None,
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
        execution_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        kind,
        attempt: 1,
        parent,
        timestamp,
    }
}

fn no_lookup(_: &str) -> Result<Option<FactRowMeta>, String> {
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
    use std::sync::{Arc, Barrier};
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

    #[test]
    fn test_事実行追記_単発追記中と完了後にopen_fd数が変わらない() {
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
        let worker = std::thread::spawn(move || {
            append_single_fact(&worker_store, &meta, &NodeFact::RetryRequested, 1_000)
        });
        stall.wait_until_arrived();
        let in_flight = open_fd_count();
        stall.release();
        worker.join().unwrap().unwrap();
        let after = open_fd_count();

        // Then: 追記中・完了後とも fd 数が増えず、事実行が記録される
        assert_eq!(in_flight, before);
        assert_eq!(after, before);
        let records = read_tree_records(&store, "fd-single-tree").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].meta.node_execution_id, "fd-single-node");
        assert_eq!(records[0].fact, NodeFact::RetryRequested);
    }

    #[test]
    fn test_事実行追記_全追記が並行実行中でもopen_fd数が変わらず全行を記録する() {
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
        let barrier = Arc::new(Barrier::new(APPEND_COUNT + 1));
        let workers = (0..APPEND_COUNT)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let meta = test_fact_meta("fd-parallel-tree", &format!("node-{index}"));
                    barrier.wait();
                    append_single_fact(&store, &meta, &NodeFact::RetryRequested, index as i64)
                })
            })
            .collect::<Vec<_>>();
        let before = open_fd_count();

        // When: 1本が writer に到達し、残りすべてが queue に滞留した状態で測る
        barrier.wait();
        stall.wait_until_arrived();
        wait_until_pending_request_count(&store, APPEND_COUNT - 1);
        let in_flight = open_fd_count();
        stall.release();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        let after = open_fd_count();

        // Then: 全 append の実行中・完了後とも fd 数が増えず、全行が記録される
        assert_eq!(in_flight, before);
        assert_eq!(after, before);
        let records = read_tree_records(&store, "fd-parallel-tree").unwrap();
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
    #[test]
    fn test_事実行追記_fd_soft_limit直下でもsession_attachedを記録する() {
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
        append_single_fact(&store, &warm_up_meta, &NodeFact::RetryRequested, 1_000).unwrap();
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
        append_single_fact(&store, &meta, &fact, 2_000).unwrap();

        // Then: fd を追加取得せず追記でき、同じ事実行を既存 reader から読める
        let records = read_tree_records(&store, "fd-soft-limit-tree").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].meta.node_execution_id, "fd-soft-limit-node");
        assert_eq!(records[0].fact, fact);
    }
}

mod append_contract_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::writer::NORMAL_LANE_MAX_BYTES;

    fn read_raw_rows(
        store: &Arc<LocalEventStore>,
        tree_id: &str,
    ) -> Vec<crate::adaptor::gateway::local_event_store::node_events::NodeEventRow> {
        let tree_id = tree_id.to_string();
        store
            .submit_indexed_query_blocking(move |connection| {
                node_events::read_tree(connection, &tree_id)
                    .map_err(|_| LocalEventQueryError::InvalidRequest)
            })
            .unwrap()
    }

    #[test]
    fn test_事実行追記_同期文脈で記録され結果が返る() {
        // Given: 同期文脈で利用する file-backed store と単独の事実
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        let meta = test_fact_meta("sync-context-tree", "sync-context-node");

        // When: 事実行を追記する
        let result = append_single_fact(&store, &meta, &NodeFact::RetryRequested, 1_000);

        // Then: 結果が返り、事実行が記録される
        assert_eq!(result, Ok(()));
        let records = read_tree_records(&store, "sync-context-tree").unwrap();
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

        // When: runtime worker 上から同期 append を呼ぶ
        let result = append_single_fact(&store, &meta, &NodeFact::ResumeRequested, 2_000);

        // Then: 呼び出しが停止せず結果が返り、事実行が記録される
        assert_eq!(result, Ok(()));
        let records = read_tree_records(&store, "async-context-tree").unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fact, NodeFact::ResumeRequested);
    }

    #[test]
    fn test_事実行追記_同一nodeの内容とseqが入力順に記録される() {
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

        // When: 3行を1行ずつ同期 append する
        append_pending_rows_blocking(&store, rows).unwrap();

        // Then: NewNodeEventRow の全 field・timestamp・払い出し seq が入力順と一致する
        let stored = read_raw_rows(&store, "ordering-tree");
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

    #[test]
    fn test_事実行追記_利用不能な追記先の失敗が呼び出し元へ返る() {
        // Given: write queue が閉じた store
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();
        store.close_write_queue_for_tests();
        let meta = test_fact_meta("unavailable-tree", "unavailable-node");

        // When: 事実行を追記する
        let error =
            append_single_fact(&store, &meta, &NodeFact::AbortRequested, 1_000).unwrap_err();

        // Then: 失敗が握りつぶされず呼び出し元へ返り、行は記録されない
        assert_eq!(
            error,
            "node fact append failed: node event write outcome is unknown"
        );
        assert!(read_raw_rows(&store, "unavailable-tree").is_empty());
    }

    #[test]
    fn test_事実行追記_複数行の途中失敗で前の行だけが記録される() {
        // Given: 正常行、queue 容量を超える行、未投入で終わる正常行の順の入力
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
        let error = append_pending_rows_blocking(&store, vec![before, failed, after]).unwrap_err();

        // Then: 容量拒否が返り、成功済みの1行だけが durable のまま残る
        assert_eq!(
            error,
            "node fact append failed: node event storage is unavailable"
        );
        let stored = read_raw_rows(&store, "partial-tree");
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
    fn test_写像_遷移イベントは行にならない() {
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

        // Then: started の1行だけが残る
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.event_type, "started");
    }

    #[test]
    fn test_写像_commandの完了と失敗はprocess_exitedになる() {
        // Given: command node の完了と（別 attempt の）失敗
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
            WorkflowEvent::NodeFailed {
                execution_id: TREE.to_string(),
                node_execution_id: "c-exec".to_string(),
                node_name: "run".to_string(),
                attempt: 1,
                reason: "exit 1".to_string(),
                failure_kind: crate::domain::workflow::NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
                timestamp: 3.0,
            },
        ];

        // When
        let rows = fact_rows_for_events(&events, no_lookup, no_lookup).unwrap();

        // Then: process_exited が2行（成功は exit 0、失敗は failure 情報つき）
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].row.event_type, "process_exited");
        assert!(rows[1].row.detail.contains("\"exitCode\":0"));
        assert_eq!(rows[2].row.event_type, "process_exited");
        assert!(rows[2].row.detail.contains("validation_failure"));
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
    use crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository;
    use crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository;
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
    use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;
    use crate::domain::workflow::{
        IsolatedWorktreeLedgerRepository, RepositoryWorktreeInventory, RuntimeExecutionState,
    };
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

    fn row_count(store: &std::sync::Arc<LocalEventStore>) -> usize {
        read_tree_records(store, TREE).unwrap().len()
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
        .unwrap();
        assert!(!read_tree_records(&store, session_id)
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));

        let mut new_id = test_id_source();
        let reconciliation = reconcile_tree_pass(&store, session_id, 10.0, &mut new_id, None)
            .unwrap()
            .unwrap();

        assert!(reconciliation.leaves.is_empty());
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
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
        let node = SqliteWorkspaceTreeRepository::new(store)
            .load_node(&WorkspaceIdentity::new("/repo"), session_id)
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
    #[test]
    fn test_再入_完了と次の開始の間でkillされた前進を検出して継続する() {
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
        .unwrap();
        let before = row_count(&store);

        // When: reconciliation パスを実行する
        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id, None)
            .unwrap()
            .unwrap();

        // Then: 次の子（run command）の started が追記され、起動対象として返る
        assert_eq!(outcome.leaves.len(), 1);
        assert_eq!(outcome.leaves[0].node_name, "run");
        let records = read_tree_records(&store, TREE).unwrap();
        assert_eq!(records.len(), before + 1);
        let last = records.last().unwrap();
        assert_eq!(last.fact.event_type(), "started");
        assert_eq!(last.meta.node_name, "run");

        // Then: started だけが永続化された kill 点では同じ leaf を再び起動対象に返し、
        // started を重複して追記しない。
        let mut new_id = test_id_source();
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(second.leaves, outcome.leaves);
        let started_rows = |records: &[crate::domain::workflow::NodeFactRecord]| {
            records
                .iter()
                .filter(|record| record.fact.event_type() == "started")
                .count()
        };
        let after_second = read_tree_records(&store, TREE).unwrap();
        assert_eq!(after_second.len(), records.len());
        assert_eq!(started_rows(&after_second), started_rows(&records));

        // 実プロセスの spawn 事実まで存在する場合だけ、次回起動時に喪失として扱う。
        append_facts_for_events(
            &store,
            &[WorkflowEvent::CommandSpawned {
                execution_id: TREE.to_string(),
                node_execution_id: outcome.leaves[0].node_execution_id.clone(),
                display_command: "true".to_string(),
                timestamp: 11.5,
            }],
        )
        .unwrap();
        let mut new_id = test_id_source();
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert!(third.leaves.is_empty());
        let after_third = read_tree_records(&store, TREE).unwrap();
        assert_eq!(
            after_third.last().unwrap().fact.event_type(),
            "process_exited"
        );

        // 喪失記録後は安定点に達する。
        let count_after_third = after_third.len();
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store), count_after_third);
    }

    /// kill 点: 合成子の started と実効 entry の子の started の間。
    #[test]
    fn test_再入_entry未開始のsequenceに実効entryの開始を補完する() {
        let (_root, store) = open_store();
        append_facts_for_events(
            &store,
            &[
                started_event(),
                node_started("main-exec", "main", NodeKindName::Sequence, None, 1.0),
            ],
        )
        .unwrap();

        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id, None)
            .unwrap()
            .unwrap();

        // Then: entry の子 a が開始される
        assert_eq!(outcome.leaves.len(), 1);
        assert_eq!(outcome.leaves[0].node_name, "a");
        let records = read_tree_records(&store, TREE).unwrap();
        assert_eq!(records.last().unwrap().meta.node_name, "a");

        // provider lifecycle の準備は node_events 上の外部実行成立事実ではない。
        // attach 前に kill された場合、2周目も同じ leaf を返し、started は増やさない。
        let before_second = read_tree_records(&store, TREE).unwrap();
        let expected_record_count = before_second.len();
        let started_count = before_second
            .iter()
            .filter(|record| record.fact.event_type() == "started")
            .count();
        let mut new_id = test_id_source();
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(second.leaves, outcome.leaves);
        let after_second = read_tree_records(&store, TREE).unwrap();
        assert_eq!(after_second.len(), expected_record_count);
        assert_eq!(
            after_second
                .iter()
                .filter(|record| record.fact.event_type() == "started")
                .count(),
            started_count
        );

        append_facts_for_events(
            &store,
            &[WorkflowEvent::SessionAttached {
                execution_id: TREE.to_string(),
                node_execution_id: outcome.leaves[0].node_execution_id.clone(),
                session_id: "session-1".to_string(),
                timestamp: 11.5,
            }],
        )
        .unwrap();
        let mut new_id = test_id_source();
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert!(third.leaves.is_empty());
        let after_third = read_tree_records(&store, TREE).unwrap();
        assert_eq!(
            after_third.last().unwrap().fact.event_type(),
            "process_exited"
        );

        let count_after_third = after_third.len();
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store), count_after_third);
    }

    /// kill 点: 実行中プロセスごと落ちた場合。喪失の観測が追記され Paused が
    /// 導出される（復旧専用の遷移イベントは書かれない）。
    #[test]
    fn test_再入_実行中プロセスの喪失を観測として追記しpausedを導出する() {
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
        .unwrap();

        let mut new_id = test_id_source();
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id, None)
            .unwrap()
            .unwrap();

        // Then: process_exited（喪失）が追記され、node は Paused・木は Running
        assert!(outcome.leaves.is_empty());
        let records = read_tree_records(&store, TREE).unwrap();
        assert_eq!(records.last().unwrap().fact.event_type(), "process_exited");
        assert_eq!(
            outcome
                .folded
                .aggregate
                .node_execution("a-exec")
                .map(|node| node.status),
            Some(RuntimeNodeExecutionStatus::Paused)
        );
        assert_eq!(
            *outcome.folded.aggregate.state(),
            RuntimeExecutionState::Running
        );

        // 冪等: Paused は喪失対象でないため2周目は何も追記しない
        let count = row_count(&store);
        let mut new_id = test_id_source();
        reconcile_tree_pass(&store, TREE, 11.0, &mut new_id, None)
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store), count);
    }

    #[test]
    fn test_隔離worktree喪失を同じpassで一度だけ記録しleafを再起動しない() {
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
        .unwrap();
        let meta = NodeFactMeta {
            tree_id: TREE.to_string(),
            node_execution_id: "a-exec".to_string(),
            parent_id: Some("main-exec".to_string()),
            node_name: "a".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
        };
        append_single_fact(
            &store,
            &meta,
            &NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                repository_root: "/repo".to_string(),
                worktree_path: "/repo-worktrees/.releash-isolated/a-exec-a1".to_string(),
                branch: "releash/isolated/a-exec-a1".to_string(),
            }),
            3,
        )
        .unwrap();
        let ledger = NodeEventIsolatedWorktreeLedgerRepository::new(store.clone());
        ledger.snapshot().unwrap();
        let inventory = [RepositoryWorktreeInventory::new("/repo", Vec::new())];

        let mut new_id = test_id_source();
        let first = reconcile_tree_pass(
            &store,
            TREE,
            10.0,
            &mut new_id,
            Some(WorktreeReconciliationPorts {
                ledger: &ledger,
                inventory: &inventory,
            }),
        )
        .unwrap()
        .unwrap();
        assert!(first.leaves.is_empty());
        assert_eq!(
            first
                .folded
                .isolated_worktrees
                .recovery_cause_for_node(TREE, "a-exec")
                .unwrap()
                .to_string(),
            "isolated worktree is missing: /repo-worktrees/.releash-isolated/a-exec-a1"
        );
        assert!(!read_tree_records(&store, TREE)
            .unwrap()
            .iter()
            .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));

        let count = row_count(&store);
        let mut new_id = test_id_source();
        reconcile_tree_pass(
            &store,
            TREE,
            11.0,
            &mut new_id,
            Some(WorktreeReconciliationPorts {
                ledger: &ledger,
                inventory: &inventory,
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(row_count(&store), count);
    }
}

mod round_trip_tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
    #[test]
    fn test_session起動由来seedはrootとattachmentを同じdurable_batchで記録する() {
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
        )
        .unwrap()
        .into_facts();
        store.fault_injector().arm_fail_after_participant_write(1);

        // When: root 書き込み直後に batch を失敗させる
        let failed = append_fact_batch_for_seed(&store, &facts, 1, "session-seed-atomic");

        // Then: root だけの中間状態は durable にならず、同じ batch を再試行できる
        assert!(failed.is_err());
        assert!(read_tree_records(&store, session_id).unwrap().is_empty());
        append_fact_batch_for_seed(&store, &facts, 1, "session-seed-atomic").unwrap();
        let records = read_tree_records(&store, session_id).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].fact.event_type(), "started");
        assert_eq!(records[1].fact.event_type(), "session_attached");
    }

    /// エンジンが発するイベント列を写像して append した事実ログが、
    /// fold で同じ実行木として導出されることの統合確認。
    #[test]
    fn test_store経由_写像した事実ログをfoldすると実行木が導出される() {
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
            append_facts_for_events(&store, batch).unwrap();
        }
        let records = read_tree_records(&store, TREE).unwrap();
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
        let root = &tree.root;
        assert_eq!(
            root.definition
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
                record.fact.event_type(),
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
                    | "abort_requested"
                    | "archive_requested"
                    | "restore_requested"
            ));
        }
    }
}
