use crate::adaptor::gateway::workflow::fact_codec;
use std::collections::HashMap;

use super::*;
use crate::domain::workflow::services::fact_replay::fold_execution_tree;
use crate::domain::workflow::value_objects::ContractViolationRecord;
use crate::domain::workflow::{
    ChildEntry, ExecutionOrigin, ExecutionParentRef, NodeDefinition, NodeKind, SequenceSpec,
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
            failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
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
            failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
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
            failure_kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
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

#[test]
fn test_fact読み出し_呼び出し境界で混雑と期限切れと破損の分類を保持する() {
    // Given
    for source in [
        LocalEventQueryError::QueryBusy,
        LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
            message: "deadline exceeded".into(),
        }),
        LocalEventQueryError::Corrupt {
            correlation_id: "id".into(),
        },
        LocalEventQueryError::Internal {
            correlation_id: "id".into(),
        },
    ] {
        // When
        let workspace = LocalEventQueryError::from(FactReadError::Query(source.clone()));
        let archive =
            crate::domain::workflow::WorkflowError::from(FactReadError::Query(source.clone()));
        // Then
        assert_eq!(workspace, source);
        assert!(
            matches!(archive, crate::domain::workflow::WorkflowError::Store(failure)
            if matches!(&failure.source, crate::domain::failure::StorageFailureSource::Query(value) if value == &source))
        );
    }
    for message in ["invalid fact", "missing session attachment"] {
        // When
        let workspace = LocalEventQueryError::from(FactReadError::Corrupt(message.into()));
        let archive =
            crate::domain::workflow::WorkflowError::from(FactReadError::Corrupt(message.into()));
        // Then
        assert!(matches!(workspace, LocalEventQueryError::Corrupt { .. }));
        assert!(matches!(archive,
            crate::domain::workflow::WorkflowError::CorruptStoredState(value) if value == message));
    }
}
