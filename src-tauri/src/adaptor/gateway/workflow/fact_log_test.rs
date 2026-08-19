use std::collections::HashMap;

use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::workflow::services::fact_replay::fold_execution_tree;
use crate::domain::workflow::value_objects::ContractViolationRecord;
use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, NodeDefinition, NodeKind, RuntimeExecutionState, SequenceSpec,
    WorkflowDefinition,
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
                }),
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
        assert!(rows[0].row.detail.contains("\"tree\":\"workflow\""));
        assert_eq!(rows[1].row.parent_id.as_deref(), Some("main-exec"));
        assert!(!rows[1].row.detail.contains("\"tree\""));
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
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
    use crate::domain::workflow::RuntimeExecutionState;

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
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
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
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
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
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id)
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
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id)
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
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
            .unwrap()
            .unwrap();

        // Then: entry の子 a が開始される
        assert_eq!(outcome.leaves.len(), 1);
        assert_eq!(outcome.leaves[0].node_name, "a");
        let records = read_tree_records(&store, TREE).unwrap();
        assert_eq!(records.last().unwrap().meta.node_name, "a");

        // provider lifecycle の準備は node_events 上の外部実行成立事実ではない。
        // attach 前に kill された場合、2周目も同じ leaf を返し、started は増やさない。
        let started_count = read_tree_records(&store, TREE)
            .unwrap()
            .iter()
            .filter(|record| record.fact.event_type() == "started")
            .count();
        let mut new_id = test_id_source();
        let second = reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
            .unwrap()
            .unwrap();
        assert_eq!(second.leaves, outcome.leaves);
        let after_second = read_tree_records(&store, TREE).unwrap();
        assert_eq!(after_second.len(), started_count);
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
        let third = reconcile_tree_pass(&store, TREE, 12.0, &mut new_id)
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
        reconcile_tree_pass(&store, TREE, 13.0, &mut new_id)
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
        let outcome = reconcile_tree_pass(&store, TREE, 10.0, &mut new_id)
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
        reconcile_tree_pass(&store, TREE, 11.0, &mut new_id)
            .unwrap()
            .unwrap();
        assert_eq!(row_count(&store), count);
    }
}

mod round_trip_tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;

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
