use super::*;
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
use crate::domain::workflow::{
    ApprovalGrantedFact, ChildEntry, CommandSpec, ExecutionOrigin, ExecutionParentRef, FanoutSpec,
    NodeCompletion, NodeFactMeta, NodeKind, OnFailure, RuntimeExecutionState, SequenceSpec,
    SessionAttachedFact, StartedFact, StopReceivedFact, SubmitReceivedFact, WorkflowRootFact,
};

const TREE: &str = "tree-1";

struct FactLog {
    seq: i64,
    records: Vec<NodeFactRecord>,
}

impl FactLog {
    fn new() -> Self {
        Self {
            seq: 0,
            records: Vec::new(),
        }
    }

    fn push(&mut self, meta: NodeFactMeta, fact: NodeFact) {
        self.seq += 1;
        self.records.push(NodeFactRecord {
            meta,
            seq: self.seq,
            timestamp_ms: self.seq * 1000,
            fact,
        });
    }
}

fn meta(
    node_execution_id: &str,
    parent_id: Option<&str>,
    node_name: &str,
    kind: NodeKindName,
    attempt: u32,
) -> NodeFactMeta {
    NodeFactMeta {
        tree_id: TREE.to_string(),
        node_execution_id: node_execution_id.to_string(),
        parent_id: parent_id.map(str::to_string),
        node_name: node_name.to_string(),
        kind,
        attempt,
    }
}

fn session_leaf(name: &str) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        ..NodeDefinition::default()
    }
}

fn command_leaf(name: &str) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Command(CommandSpec {
            command: "true".to_string(),
        }),
        ..NodeDefinition::default()
    }
}

fn sequence_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Sequence(SequenceSpec {
            entry: None,
            output: None,
            children,
        }),
        ..NodeDefinition::default()
    }
}

fn fanout_node(name: &str, children: Vec<ChildEntry>) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children,
            items: None,
        }),
        ..NodeDefinition::default()
    }
}

fn workflow_definition(nodes: Vec<NodeDefinition>, entry: &str) -> WorkflowDefinition {
    WorkflowDefinition {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes,
        entry: entry.to_string(),
    }
}

fn workflow_root(definition: WorkflowDefinition) -> TreeRootFact {
    TreeRootFact::Workflow(WorkflowRootFact {
        workflow_name: definition.name.clone(),
        worktree_path: "/repo".to_string(),
        created_from: ExecutionOrigin::Cli,
        request: "please work".to_string(),
        definition,
    })
}

fn session_root() -> TreeRootFact {
    TreeRootFact::Session(SessionRootFact {
        workspace_identity: "/repo".to_string(),
        worktree_path: "/repo".to_string(),
        session: crate::domain::workflow::SessionSpec::default(),
        created_from: ExecutionOrigin::DesktopUi,
    })
}

fn started_root(root: TreeRootFact) -> NodeFact {
    NodeFact::Started(StartedFact {
        parent: None,
        root: Some(root),
    })
}

fn started_child(parent: ExecutionParentRef) -> NodeFact {
    NodeFact::Started(StartedFact {
        parent: Some(parent),
        root: None,
    })
}

fn attached(session_id: &str) -> NodeFact {
    NodeFact::SessionAttached(SessionAttachedFact {
        session_id: session_id.to_string(),
        provider_session_id: None,
        transcript_ref: None,
        initial_instruction_admitted: false,
    })
}

fn submit() -> NodeFact {
    NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None })
}

fn stop() -> NodeFact {
    NodeFact::StopReceived(StopReceivedFact {
        result_summary: None,
        token_usage: None,
    })
}

fn stop_with_summary(summary: &str) -> NodeFact {
    NodeFact::StopReceived(StopReceivedFact {
        result_summary: Some(summary.to_string()),
        token_usage: None,
    })
}

fn exited(code: i32) -> NodeFact {
    NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
        exit_code: Some(code),
        result_summary: None,
        failure_reason: None,
        failure_kind: None,
    })
}

fn process_lost() -> NodeFact {
    NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
        exit_code: None,
        result_summary: None,
        failure_reason: None,
        failure_kind: None,
    })
}

fn node_status(tree: &FoldedTree, node_execution_id: &str) -> RuntimeNodeExecutionStatus {
    tree.aggregate
        .node_execution(node_execution_id)
        .expect("node execution must exist")
        .status
}

mod standalone_session_tests {
    use super::*;

    #[test]
    fn test_単独session_startedの追記だけで1ノードの実行木として導出される() {
        // Given: 単独 session の root started のみ
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta, attached("session-1"));

        // When: fold する
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 1 ノードの実行木が Running で導出される
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
        assert_eq!(tree.aggregate.node_executions().len(), 1);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Running
        );
        assert!(matches!(tree.root, TreeRootFact::Session(_)));
    }

    #[test]
    fn test_単独session_submitとstopの二信号で完了が導出される() {
        // Given: 完了二信号まで揃った事実列（遷移イベントは存在しない）
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), submit());
        log.push(root_meta, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 完了は事実からの導出のみで決まる
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn test_単独session_stopが運ぶ結果summaryがread_modelへ導出される() {
        // Given: 親スコープを持たない root leaf の stop が result summary を運ぶ
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), submit());
        log.push(
            root_meta,
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: Some("summarized".to_string()),
                token_usage: None,
            }),
        );

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let model = derive_read_model(&tree);

        // Then: root leaf でも result_summary が失われない
        assert_eq!(
            model.node_executions[0].result_summary.as_deref(),
            Some("summarized")
        );
        assert_eq!(
            model.status,
            crate::domain::workflow::ExecutionStatus::Completed
        );
    }

    #[test]
    fn test_単独session_archiveとrestoreが最終状態として導出される() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), NodeFact::ArchiveRequested);
        assert!(super::derive_session_facts(&log.records, "root-exec", "root-exec").archived);

        log.push(root_meta, NodeFact::RestoreRequested);
        assert!(!super::derive_session_facts(&log.records, "root-exec", "root-exec").archived);
    }

    #[test]
    fn test_単独session_process_exitの状態と異常終了を導出する() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), exited(0));
        let normal = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(normal.exited);
        assert!(!normal.last_exit_abnormal);

        log.push(root_meta.clone(), attached("session-1"));
        let reattached = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(!reattached.exited);

        log.push(root_meta.clone(), exited(1));
        let non_zero = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(non_zero.exited);
        assert!(non_zero.last_exit_abnormal);

        log.push(
            root_meta,
            NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
                exit_code: Some(0),
                result_summary: None,
                failure_reason: Some("provider failure".to_string()),
                failure_kind: None,
            }),
        );
        let failed = super::derive_session_facts(&log.records, "root-exec", "session-1");
        assert!(failed.last_exit_abnormal);
    }
}

mod sequence_tests {
    use super::*;

    fn two_step_definition() -> WorkflowDefinition {
        workflow_definition(
            vec![
                session_leaf("a"),
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![ChildEntry::reference("a"), ChildEntry::reference("b")],
                ),
            ],
            "main",
        )
    }

    #[test]
    fn test_sequence_子の完了導出と前進の事実で終端到達が完了になる() {
        // Given: a → b と進んだ workflow の事実列
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(two_step_definition())),
        );
        let a = meta("a-exec", Some("main-exec"), "a", NodeKindName::Session, 1);
        log.push(
            a.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(a.clone(), attached("session-a"));
        log.push(a.clone(), submit());
        log.push(a, stop());
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), attached("session-b"));
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: a / b とも完了、sequence は終端到達で完了、木全体も完了
        assert_eq!(
            node_status(&tree, "a-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            node_status(&tree, "b-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            node_status(&tree, "main-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_sequence_前半のみの事実列では実行中のままになる() {
        // Given: a 完了までの事実列（b は未開始）
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(two_step_definition())),
        );
        let a = meta("a-exec", Some("main-exec"), "a", NodeKindName::Session, 1);
        log.push(
            a.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(a.clone(), submit());
        log.push(a, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: a は完了・木は実行中（前進は事実が無い限り起きない）
        assert_eq!(
            node_status(&tree, "a-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
    }
}

mod fanout_tests {
    use super::*;

    #[test]
    fn test_fanout_全子完了で合成子と木全体の完了が導出される() {
        // Given: fanout の 2 子が完了した事実列
        let definition = workflow_definition(
            vec![
                session_leaf("x"),
                session_leaf("y"),
                fanout_node(
                    "fan",
                    vec![ChildEntry::reference("x"), ChildEntry::reference("y")],
                ),
                sequence_node("main", vec![ChildEntry::reference("fan")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        log.push(
            meta(
                "fan-exec",
                Some("main-exec"),
                "fan",
                NodeKindName::Fanout,
                1,
            ),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        let x = meta("x-exec", Some("fan-exec"), "x", NodeKindName::Session, 1);
        log.push(
            x.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 0)),
        );
        let y = meta("y-exec", Some("fan-exec"), "y", NodeKindName::Session, 1);
        log.push(
            y.clone(),
            started_child(ExecutionParentRef::fanout_child("fan-exec", None, 1)),
        );
        log.push(x.clone(), submit());
        log.push(x, stop());

        // 1 子完了時点では fanout は未完了
        let halfway = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&halfway, "fan-exec"),
            RuntimeNodeExecutionStatus::Running
        );

        log.push(y.clone(), submit());
        log.push(y, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: 全子完了 → fanout 完了 → sequence 終端 → 木完了
        assert_eq!(
            node_status(&tree, "fan-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_fanout_同名同attemptの並走laneで結果summaryを取り違えない() {
        let definition = workflow_definition(
            vec![
                session_leaf("worker"),
                fanout_node(
                    "fan",
                    vec![
                        ChildEntry::reference("worker"),
                        ChildEntry::reference("worker"),
                    ],
                ),
                sequence_node("main", vec![ChildEntry::reference("fan")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        log.push(
            meta(
                "fan-exec",
                Some("main-exec"),
                "fan",
                NodeKindName::Fanout,
                1,
            ),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        for (index, id) in [(0, "worker-a"), (1, "worker-b")] {
            let worker = meta(id, Some("fan-exec"), "worker", NodeKindName::Session, 1);
            log.push(
                worker.clone(),
                started_child(ExecutionParentRef::fanout_child("fan-exec", None, index)),
            );
        }
        for (id, summary) in [("worker-a", None), ("worker-b", Some("result-b"))] {
            let worker = meta(id, Some("fan-exec"), "worker", NodeKindName::Session, 1);
            log.push(worker.clone(), submit());
            log.push(worker, summary.map_or_else(stop, stop_with_summary));
        }

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        let model = derive_read_model(&tree);

        for (id, expected) in [("worker-a", None), ("worker-b", Some("result-b"))] {
            assert_eq!(
                model
                    .node_executions
                    .iter()
                    .find(|node| node.id == id)
                    .and_then(|node| node.result_summary.as_deref()),
                expected
            );
        }
    }
}

mod approval_tests {
    use super::*;

    fn approval_definition() -> WorkflowDefinition {
        let mut reviewed = session_leaf("reviewed");
        reviewed.completion = NodeCompletion::Approval;
        workflow_definition(
            vec![
                reviewed,
                sequence_node("main", vec![ChildEntry::reference("reviewed")]),
            ],
            "main",
        )
    }

    #[test]
    fn test_approval_二信号が揃っても承認事実まで完了しない() {
        // Given: completion: approval の node が二信号まで揃った事実列
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(approval_definition())),
        );
        let node = meta(
            "r-exec",
            Some("main-exec"),
            "reviewed",
            NodeKindName::Session,
            1,
        );
        log.push(
            node.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(node.clone(), submit());
        log.push(node.clone(), stop());

        // When / Then: 承認待ちの導出
        let waiting = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&waiting, "r-exec"),
            RuntimeNodeExecutionStatus::WaitingApproval
        );
        assert_eq!(*waiting.aggregate.state(), RuntimeExecutionState::Running);

        // When: approval_granted の追記
        log.push(
            node,
            NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
        );
        let approved = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: human の承認事実で完了が導出される
        assert_eq!(
            node_status(&approved, "r-exec"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(
            *approved.aggregate.state(),
            RuntimeExecutionState::Completed
        );
    }
}

mod failure_tests {
    use super::*;

    #[test]
    fn test_on_failure_retry_失敗とretry事実の再実行で完了に到達する() {
        // Given: command が失敗 → retry → 成功した事実列
        let definition = workflow_definition(
            vec![
                command_leaf("c"),
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![
                        ChildEntry {
                            name: "c".to_string(),
                            inputs: Vec::new(),
                            rules: None,
                            on_failure: Some(OnFailure::Retry(1)),
                        },
                        ChildEntry::reference("b"),
                    ],
                ),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        let first = meta("c-exec-1", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            first.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(first.clone(), exited(1));
        log.push(first, NodeFact::RetryRequested);
        let second = meta("c-exec-2", Some("main-exec"), "c", NodeKindName::Command, 2);
        log.push(
            second.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(second, exited(0));
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: attempt 1 は失敗のまま行として残り、attempt 2 の完了で前進した
        assert_eq!(
            node_status(&tree, "c-exec-1"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(
            node_status(&tree, "c-exec-2"),
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_on_failure_ignore_失敗のまま親の前進が導出される() {
        // Given: on_failure: ignore の command が失敗し、次の子が完了した事実列
        let definition = workflow_definition(
            vec![
                command_leaf("c"),
                session_leaf("b"),
                sequence_node(
                    "main",
                    vec![
                        ChildEntry {
                            name: "c".to_string(),
                            inputs: Vec::new(),
                            rules: None,
                            on_failure: Some(OnFailure::Ignore),
                        },
                        ChildEntry::reference("b"),
                    ],
                ),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        let c = meta("c-exec", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            c.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(c, exited(1));
        let b = meta("b-exec", Some("main-exec"), "b", NodeKindName::Session, 1);
        log.push(
            b.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(b.clone(), submit());
        log.push(b, stop());

        // When
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: c は失敗のまま、b の完了で木全体は完了
        assert_eq!(
            node_status(&tree, "c-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn test_失敗既定_on_failure宣言なしは失敗で停止したままになる() {
        let definition = workflow_definition(
            vec![
                command_leaf("c"),
                sequence_node("main", vec![ChildEntry::reference("c")]),
            ],
            "main",
        );
        let mut log = FactLog::new();
        log.push(
            meta("main-exec", None, "main", NodeKindName::Sequence, 1),
            started_root(workflow_root(definition)),
        );
        let c = meta("c-exec", Some("main-exec"), "c", NodeKindName::Command, 1);
        log.push(
            c.clone(),
            started_child(ExecutionParentRef::sequence_child("main-exec")),
        );
        log.push(c, exited(1));

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&tree, "c-exec"),
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Running);
    }
}

mod paused_tests {
    use super::*;

    #[test]
    fn test_paused_プロセス喪失は導出でありpause事実は存在しない() {
        // Given: 二信号未揃いのままプロセスが消えた session
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta.clone(), process_lost());

        // When / Then: Paused は process_exited と二信号未揃いからの純導出
        let paused = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(
            node_status(&paused, "root-exec"),
            RuntimeNodeExecutionStatus::Paused
        );
        assert_eq!(*paused.aggregate.state(), RuntimeExecutionState::Running);

        // When: resume の指示と再 attach
        log.push(root_meta.clone(), NodeFact::ResumeRequested);
        log.push(root_meta, attached("session-2"));
        let resumed = fold_execution_tree(TREE, &log.records).unwrap().unwrap();

        // Then: Running へ戻る
        assert_eq!(
            node_status(&resumed, "root-exec"),
            RuntimeNodeExecutionStatus::Running
        );
    }
}

mod abort_tests {
    use super::*;

    #[test]
    fn test_abort_指示の事実だけで中止が導出される() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), attached("session-1"));
        log.push(root_meta, NodeFact::AbortRequested);

        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Aborted);
        assert_eq!(
            node_status(&tree, "root-exec"),
            RuntimeNodeExecutionStatus::Aborted
        );
    }
}

mod retroactive_interpretation_tests {
    use super::*;

    /// 許容済みトレードオフの固定: 「当時完了と判定した」という記録は持たず、
    /// 完了は fold 時点の規則で毎回導出される。規則が変われば同じログの解釈も
    /// 変わる。このテストは、完了・遷移を表す事実が入力に一切存在しないことと、
    /// それでも完了が導出されることを両方主張する。
    #[test]
    fn test_遡及_完了は記録ではなくfold時点の規則による導出である() {
        let mut log = FactLog::new();
        let root_meta = meta("root-exec", None, "chat", NodeKindName::Session, 1);
        log.push(root_meta.clone(), started_root(session_root()));
        log.push(root_meta.clone(), submit());
        log.push(root_meta, stop());

        // 入力の事実語彙に遷移（completed 等）は存在しない
        for record in &log.records {
            assert!(matches!(
                record.fact.event_type(),
                "started" | "submit_received" | "stop_received"
            ));
        }

        // それでも完了は導出される（規則: Submit + Stop 揃いで完了）
        let tree = fold_execution_tree(TREE, &log.records).unwrap().unwrap();
        assert_eq!(*tree.aggregate.state(), RuntimeExecutionState::Completed);
    }
}

mod input_validation_tests {
    use super::*;

    #[test]
    fn test_空の事実列は木が存在しない() {
        assert!(fold_execution_tree(TREE, &[]).unwrap().is_none());
    }

    #[test]
    fn test_別のtreeの行が混ざった入力は拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "chat", NodeKindName::Session, 1),
            started_root(session_root()),
        );
        assert!(fold_execution_tree("other-tree", &log.records).is_err());
    }

    #[test]
    fn test_先頭がstartedではない事実列を拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "chat", NodeKindName::Session, 1),
            submit(),
        );

        assert!(fold_execution_tree(TREE, &log.records).is_err());
    }

    #[test]
    fn test_先頭startedがrootを持たない事実列を拒否する() {
        let mut log = FactLog::new();
        log.push(
            meta("root-exec", None, "chat", NodeKindName::Session, 1),
            NodeFact::Started(StartedFact {
                parent: None,
                root: None,
            }),
        );

        assert!(fold_execution_tree(TREE, &log.records).is_err());
    }
}
