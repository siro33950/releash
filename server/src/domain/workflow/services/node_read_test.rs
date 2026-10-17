use super::*;
use crate::domain::workflow::{
    ExecutionOrigin, NodeCompletion, NodeFactMeta, SessionDelegate, StartedFact, StopReceivedFact,
    SubmitReceivedFact, WorkflowDefinition,
};

fn root(node: NodeDefinition) -> TreeRootFact {
    TreeRootFact {
        repository_root: None,
        workspace_identity: "/repo".into(),
        worktree_path: "/repo".into(),
        created_from: ExecutionOrigin::Cli,
        request: String::new(),
        workflow_name: "workflow".into(),
        definition: Some(WorkflowDefinition {
            entry: node.name.clone(),
            nodes: vec![node],
            ..Default::default()
        }),
        launched_as: ExecutionTreeLaunch::Workflow,
    }
}

fn record(seq: i64, fact: NodeFact) -> NodeFactRecord {
    NodeFactRecord {
        meta: NodeFactMeta {
            tree_id: "tree".into(),
            node_execution_id: "node".into(),
            parent_id: None,
            node_name: "session".into(),
            kind: NodeKindName::Session,
            attempt: 1,
        },
        seq,
        timestamp_ms: seq * 1000,
        fact,
    }
}

#[test]
fn test_会話詳細の完了信号_片方では稼働を保ち二信号と承認の共通規則で決着する() {
    // Given
    for approval in [false, true] {
        let root = root(NodeDefinition {
            name: "session".into(),
            completion: NodeCompletion {
                require: approval
                    .then_some(crate::domain::workflow::CompletionRequirement::Approval),
                ..Default::default()
            },
            ..Default::default()
        });
        let mut records = vec![record(
            1,
            NodeFact::Started(StartedFact {
                root: None,
                parent: None,
                worktree: None,
            }),
        )];
        // When / Then
        for (fact, expected_signal, expected_status) in [
            (
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
                NodeCompletionSignalState::StopReceived,
                NodeExecutionStatus::Running,
            ),
            (
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                NodeCompletionSignalState::Ready,
                if approval {
                    NodeExecutionStatus::WaitingApproval
                } else {
                    NodeExecutionStatus::Succeeded
                },
            ),
        ] {
            records.push(record(records.len() as i64 + 1, fact));
            let read = subtree_read_rules(&root, &records, None).unwrap();
            assert_eq!(read["node"].completion_signals, expected_signal);
            assert_eq!(read["node"].status, expected_status);
            let folded = super::super::fact_replay::fold_execution_tree("tree", &{
                let mut records = records.clone();
                if let NodeFact::Started(started) = &mut records[0].fact {
                    started.root = Some(Box::new(root.clone()));
                }
                records
            })
            .unwrap()
            .unwrap();
            assert_eq!(
                read["node"].status,
                folded.aggregate.node_executions[0].status
            );
        }
        // When
        if approval {
            records.push(record(
                4,
                NodeFact::ApprovalGranted(crate::domain::workflow::ApprovalGrantedFact {
                    comment: None,
                }),
            ));
            // Then
            assert_eq!(
                subtree_read_rules(&root, &records, None).unwrap()["node"].status,
                NodeExecutionStatus::Succeeded
            );
        }
    }
}

#[test]
fn test_会話詳細のartifact_delegateの判定に必要な項目だけを指定する() {
    // Given
    let mut root = root(NodeDefinition {
        name: "session".into(),
        completion: NodeCompletion {
            delegate: Some(SessionDelegate {
                child: "child".into(),
                inputs: Vec::new(),
                max_iterations: 2,
                when: Predicate::And(vec![
                    Predicate::Ref("ready".into()),
                    Predicate::Ref("child.done".into()),
                ]),
            }),
            ..Default::default()
        },
        ..Default::default()
    });
    use crate::domain::workflow::{
        ChildEntry, FanoutSpec, FieldPath, ItemsSource, NodeKind, Rule, SequenceSpec,
    };
    let mut when = ChildEntry::reference("session");
    when.rules = Some(vec![Rule::When {
        on: Predicate::Ref("accept".into()),
        then: "next".into(),
        next: "other".into(),
    }]);
    let mut switch = ChildEntry::reference("next");
    switch.rules = Some(vec![Rule::Switch {
        on: "route".into(),
        cases: Default::default(),
        next: Some("other".into()),
    }]);
    root.definition.as_mut().unwrap().nodes.extend([
        NodeDefinition {
            name: "sequence".into(),
            kind: NodeKind::Sequence(SequenceSpec {
                entry: None,
                children: vec![when, switch],
            }),
            ..Default::default()
        },
        NodeDefinition {
            name: "fanout".into(),
            kind: NodeKind::Fanout(FanoutSpec {
                children: Vec::new(),
                items: Some(ItemsSource::ArtifactField {
                    node: "session".into(),
                    field_path: FieldPath::from_dotted("items").unwrap(),
                }),
            }),
            ..Default::default()
        },
    ]);
    // When
    let fields = artifact_fields(&root);
    // Then
    assert_eq!(
        fields,
        ["accept", "child.done", "done", "items", "ready", "route"]
    );
}

#[test]
fn test_会話詳細の子スコープ_順序と並列の完了と承認待ちを共有規則で導出する() {
    use crate::domain::workflow::{
        ChildEntry, ExecutionParentRef, FanoutSpec, NodeKind, SequenceSpec,
    };
    // Given
    for fanout in [false, true] {
        for approval in [false, true] {
            let mut root = root(NodeDefinition {
                name: "scope".into(),
                kind: if fanout {
                    NodeKind::Fanout(FanoutSpec {
                        children: vec![ChildEntry::reference("session")],
                        items: None,
                    })
                } else {
                    NodeKind::Sequence(SequenceSpec {
                        entry: None,
                        children: vec![ChildEntry::reference("session")],
                    })
                },
                completion: if approval {
                    NodeCompletion::require_approval()
                } else {
                    NodeCompletion::default()
                },
                ..Default::default()
            });
            root.definition
                .as_mut()
                .unwrap()
                .nodes
                .push(NodeDefinition {
                    name: "session".into(),
                    ..Default::default()
                });
            let mut parent = record(
                1,
                NodeFact::Started(StartedFact {
                    root: Some(Box::new(root.clone())),
                    parent: None,
                    worktree: None,
                }),
            );
            parent.meta.node_execution_id = "scope".into();
            parent.meta.node_name = "scope".into();
            parent.meta.kind = if fanout {
                NodeKindName::Fanout
            } else {
                NodeKindName::Sequence
            };
            let mut child = record(
                2,
                NodeFact::Started(StartedFact {
                    root: None,
                    parent: Some(if fanout {
                        ExecutionParentRef::fanout_child("scope", None, 0)
                    } else {
                        ExecutionParentRef::sequence_child("scope")
                    }),
                    worktree: None,
                }),
            );
            child.meta.parent_id = Some("scope".into());
            let mut submit = record(
                3,
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            );
            submit.meta.parent_id = Some("scope".into());
            let mut stop = record(
                4,
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
            );
            stop.meta.parent_id = Some("scope".into());
            let records = vec![parent, child, submit, stop];
            // When
            let read = subtree_read_rules(&root, &records, None).unwrap();
            let folded = super::super::fact_replay::fold_execution_tree("tree", &records)
                .unwrap()
                .unwrap();
            // Then
            assert_eq!(
                read["scope"].status,
                if approval {
                    NodeExecutionStatus::WaitingApproval
                } else {
                    NodeExecutionStatus::Succeeded
                }
            );
            assert_eq!(
                read["scope"].status,
                folded.aggregate.node_executions[0].status
            );
            assert_eq!(
                read["scope"].has_artifact,
                folded.aggregate.node_executions[0].artifact.is_some()
            );
        }
    }
}

fn child_record(
    seq: i64,
    id: &str,
    name: &str,
    kind: NodeKindName,
    parent: Option<crate::domain::workflow::ExecutionParentRef>,
    fact: NodeFact,
) -> NodeFactRecord {
    let mut value = record(seq, fact);
    value.meta.node_execution_id = id.into();
    value.meta.node_name = name.into();
    value.meta.kind = kind;
    value.meta.parent_id = parent.map(|parent| parent.parent_id);
    value
}

fn assert_replay(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    id: &str,
    expected: NodeExecutionStatus,
) {
    let read = subtree_read_rules(root, records, None).unwrap();
    let folded = super::super::fact_replay::fold_execution_tree("tree", records)
        .unwrap()
        .unwrap();
    let node = folded
        .aggregate
        .node_executions
        .iter()
        .find(|node| node.id == id)
        .unwrap();
    assert_eq!(read[id].status, expected);
    assert_eq!(read[id].status, node.status);
    assert_eq!(read[id].has_artifact, node.artifact.is_some());
}

#[test]
fn test_並列完了_未開始と一部完了を全完了と区別する() {
    use crate::domain::workflow::{ChildEntry, ExecutionParentRef, FanoutSpec};
    // Given
    for approval in [false, true] {
        let mut root = root(NodeDefinition {
            name: "scope".into(),
            kind: NodeKind::Fanout(FanoutSpec {
                children: vec![ChildEntry::reference("session")],
                items: Some(ItemsSource::Literal(vec![
                    serde_json::json!(0),
                    serde_json::json!(1),
                ])),
            }),
            completion: if approval {
                NodeCompletion::require_approval()
            } else {
                NodeCompletion::default()
            },
            ..Default::default()
        });
        root.definition
            .as_mut()
            .unwrap()
            .nodes
            .push(NodeDefinition {
                name: "session".into(),
                ..Default::default()
            });
        let mut records = vec![child_record(
            1,
            "scope",
            "scope",
            NodeKindName::Fanout,
            None,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(root.clone())),
                parent: None,
                worktree: None,
            }),
        )];
        for index in 0..2 {
            let id = format!("child-{index}");
            let parent = ExecutionParentRef::fanout_child("scope", Some(index), 0);
            // When
            records.push(child_record(
                records.len() as i64 + 1,
                &id,
                "session",
                NodeKindName::Session,
                Some(parent.clone()),
                NodeFact::Started(StartedFact {
                    root: None,
                    parent: Some(parent.clone()),
                    worktree: None,
                }),
            ));
            // Then
            assert_replay(&root, &records, "scope", NodeExecutionStatus::Running);
            // When
            for fact in [
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
            ] {
                records.push(child_record(
                    records.len() as i64 + 1,
                    &id,
                    "session",
                    NodeKindName::Session,
                    Some(parent.clone()),
                    fact,
                ));
            }
            // Then
            assert_replay(
                &root,
                &records,
                "scope",
                if index == 0 {
                    NodeExecutionStatus::Running
                } else if approval {
                    NodeExecutionStatus::WaitingApproval
                } else {
                    NodeExecutionStatus::Succeeded
                },
            );
        }
    }
}

#[test]
fn test_委任先の合成成果_選択項目を統合して親を承認待ちにする() {
    use crate::domain::workflow::{
        ArtifactProducedFact, ChildEntry, ExecutionParentRef, FanoutSpec, SequenceSpec,
    };
    // Given
    for fanout in [false, true] {
        let mut root = root(NodeDefinition {
            name: "session".into(),
            completion: NodeCompletion {
                delegate: Some(SessionDelegate {
                    child: "scope".into(),
                    inputs: Vec::new(),
                    max_iterations: 2,
                    when: Predicate::Ref("child.judge.passed".into()),
                }),
                ..NodeCompletion::require_approval()
            },
            ..Default::default()
        });
        root.definition.as_mut().unwrap().nodes.extend([
            NodeDefinition {
                name: "scope".into(),
                kind: if fanout {
                    NodeKind::Fanout(FanoutSpec {
                        children: vec![ChildEntry::reference("judge")],
                        items: None,
                    })
                } else {
                    NodeKind::Sequence(SequenceSpec {
                        children: vec![ChildEntry::reference("judge")],
                        entry: None,
                    })
                },
                ..Default::default()
            },
            NodeDefinition {
                name: "judge".into(),
                ..Default::default()
            },
        ]);
        assert!(artifact_fields(&root).contains(&"passed".into()));
        let delegate = ExecutionParentRef::delegate_child("node");
        let parent = if fanout {
            ExecutionParentRef::fanout_child("scope", None, 0)
        } else {
            ExecutionParentRef::sequence_child("scope")
        };
        let mut records = vec![record(
            1,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(root.clone())),
                parent: None,
                worktree: None,
            }),
        )];
        for fact in [
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: None,
                value: serde_json::json!({}),
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            records.push(record(records.len() as i64 + 1, fact));
        }
        records.push(child_record(
            5,
            "scope",
            "scope",
            if fanout {
                NodeKindName::Fanout
            } else {
                NodeKindName::Sequence
            },
            Some(delegate.clone()),
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(delegate),
                worktree: None,
            }),
        ));
        records.push(child_record(
            6,
            "judge",
            "judge",
            NodeKindName::Session,
            Some(parent.clone()),
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(parent.clone()),
                worktree: None,
            }),
        ));
        // When
        for fact in [
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                contract: None,
                value: serde_json::json!({"passed":true}),
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            records.push(child_record(
                records.len() as i64 + 1,
                "judge",
                "judge",
                NodeKindName::Session,
                Some(parent.clone()),
                fact,
            ));
        }
        // Then
        assert_replay(
            &root,
            &records,
            "node",
            NodeExecutionStatus::WaitingApproval,
        );
    }
}

#[test]
fn test_空の隔離並列_開始事実から承認待ちを導出する() {
    use crate::domain::workflow::{ChildEntry, FanoutSpec, IsolatedWorktree};
    // Given
    let mut root = root(NodeDefinition {
        name: "scope".into(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![ChildEntry::reference("session")],
            items: Some(ItemsSource::Literal(Vec::new())),
        }),
        worktree: Some(crate::domain::workflow::WorktreeMode::Isolated),
        completion: NodeCompletion::require_approval(),
        ..Default::default()
    });
    root.definition
        .as_mut()
        .unwrap()
        .nodes
        .push(NodeDefinition {
            name: "session".into(),
            ..Default::default()
        });
    root.repository_root = Some("/repo".into());
    let records = vec![child_record(
        1,
        "scope",
        "scope",
        NodeKindName::Fanout,
        None,
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root.clone())),
            parent: None,
            worktree: Some(IsolatedWorktree {
                branch: "branch".into(),
                path: "/isolated".into(),
            }),
        }),
    )];
    // When / Then
    assert_replay(
        &root,
        &records,
        "scope",
        NodeExecutionStatus::WaitingApproval,
    );
}

#[test]
fn test_空の隔離並列_起動失敗と中止時は完了せず承認後だけ成功する() {
    use crate::domain::workflow::{
        AbortRequestedFact, ApprovalGrantedFact, ChildEntry, FanoutSpec, IsolatedWorktree,
        NodeExecutionFailureKind, RuntimeFailureObservedFact, WorktreeMode,
    };
    // Given
    let mut root = root(NodeDefinition {
        name: "scope".into(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![ChildEntry::reference("session")],
            items: Some(ItemsSource::Literal(Vec::new())),
        }),
        worktree: Some(WorktreeMode::Isolated),
        completion: NodeCompletion::require_approval(),
        ..Default::default()
    });
    root.repository_root = Some("/repo".into());
    root.definition
        .as_mut()
        .unwrap()
        .nodes
        .push(NodeDefinition {
            name: "session".into(),
            ..Default::default()
        });
    let start = child_record(
        1,
        "scope",
        "scope",
        NodeKindName::Fanout,
        None,
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root.clone())),
            parent: None,
            worktree: Some(IsolatedWorktree {
                branch: "branch".into(),
                path: "/isolated".into(),
            }),
        }),
    );
    for (fact, expected) in [
        (
            NodeFact::RuntimeFailureObserved(RuntimeFailureObservedFact {
                reason: "failed".into(),
                failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            }),
            NodeExecutionStatus::Running,
        ),
        (
            NodeFact::ApprovalGranted(ApprovalGrantedFact { comment: None }),
            NodeExecutionStatus::Succeeded,
        ),
    ] {
        // When
        let records = vec![
            start.clone(),
            child_record(2, "scope", "scope", NodeKindName::Fanout, None, fact),
        ];
        // Then
        assert_replay(&root, &records, "scope", expected);
    }
    // When
    let terminal = child_record(
        2,
        "scope",
        "scope",
        NodeKindName::Fanout,
        None,
        NodeFact::AbortRequested(AbortRequestedFact::default()),
    );
    let records = vec![start, terminal.clone()];
    let read = subtree_read_rules(&root, &records, Some(&terminal)).unwrap();
    // Then
    assert_eq!(read["scope"].status, NodeExecutionStatus::Aborted);
    assert!(!read["scope"].has_artifact);
}

fn selected_artifact_records(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
) -> Vec<NodeFactRecord> {
    let fields = artifact_fields(root);
    let mut selected = records.to_vec();
    for record in &mut selected {
        if let NodeFact::ArtifactProduced(artifact) = &mut record.fact {
            let mut inputs = serde_json::Map::new();
            for field in &fields {
                let value = crate::domain::workflow::FieldPath::from_dotted(field)
                    .ok()
                    .and_then(|path| {
                        super::super::reference::resolve_value_at_path(&artifact.value, &path)
                    });
                let value = value
                    .map(|value| {
                        if let Some(items) = value.as_array() {
                            serde_json::json!(items.len())
                        } else if value.is_boolean() || value.is_string() {
                            value.clone()
                        } else {
                            serde_json::Value::Null
                        }
                    })
                    .unwrap_or(serde_json::Value::Null);
                inputs.insert(field.clone(), value);
            }
            artifact.value = serde_json::Value::Object(inputs);
        }
    }
    selected
}

#[test]
fn test_会話詳細の親再試行_旧親の委任子が完了すると新親を承認待ちにする() {
    // Given
    let records = super::super::test_helpers::retried_delegate_parent_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root");
    };
    let root = started.root.as_deref().unwrap();
    let target = records
        .iter()
        .find(|record| record.meta.attempt == 2)
        .unwrap()
        .meta
        .node_execution_id
        .as_str();
    // When / Then
    for (end, expected) in [
        (records.len() - 3, NodeExecutionStatus::Running),
        (records.len(), NodeExecutionStatus::WaitingApproval),
    ] {
        let records = &records[..end];
        let read =
            subtree_read_rules(root, &selected_artifact_records(root, records), None).unwrap();
        let execution =
            super::super::fact_replay::fold_execution_tree(&records[0].meta.tree_id, records)
                .unwrap()
                .unwrap();
        let node = execution
            .aggregate
            .node_executions
            .iter()
            .find(|node| node.id == target)
            .unwrap();
        assert_eq!(read[target].status, node.status);
        assert_eq!(read[target].completion_signals, node.completion_signals);
        assert_eq!(read[target].has_artifact, node.artifact.is_some());
        assert_eq!(read[target].status, expected);
    }
    // Then
    let read = subtree_read_rules(root, &selected_artifact_records(root, &records), None).unwrap();
    assert_eq!(read[target].status, NodeExecutionStatus::WaitingApproval);
    let child = records
        .iter()
        .find(|record| record.meta.node_name == "judge")
        .unwrap();
    assert_eq!(
        read[&child.meta.node_execution_id].parent_id.as_deref(),
        Some(target)
    );
}

#[test]
fn test_会話詳細の動的並列_別スコープの後発成果を予定件数に混ぜない() {
    // Given
    let records = super::super::test_helpers::scoped_dynamic_fanout_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root");
    };
    let root = started.root.as_deref().unwrap();
    let target = records
        .iter()
        .find(|record| record.meta.node_name == "inner")
        .unwrap()
        .meta
        .node_execution_id
        .as_str();
    for (end, expected) in [
        (records.len() - 4, NodeExecutionStatus::Running),
        (records.len(), NodeExecutionStatus::WaitingApproval),
    ] {
        // When
        let records = &records[..end];
        let selected = selected_artifact_records(root, records);
        let read = subtree_read_rules(root, &selected, None).unwrap();
        let execution =
            super::super::fact_replay::fold_execution_tree(&records[0].meta.tree_id, records)
                .unwrap()
                .unwrap();
        // Then
        assert_eq!(read[target].status, expected);
        assert_eq!(
            read[target].status,
            execution
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == target)
                .unwrap()
                .status
        );
        assert!(!format!("{selected:?}").contains("private"));
    }
}
