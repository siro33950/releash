use super::*;
use crate::domain::workflow::{
    ExecutionOrigin, NodeCompletion, NodeFactMeta, SessionDelegate, StartedFact, StopReceivedFact,
    SubmitReceivedFact, WorkflowDefinition,
};

fn read_projection(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
) -> Result<NodeReadProjectionResult, String> {
    super::subtree_read_projection(
        root,
        records,
        terminal,
        &super::super::history::retry_predecessors(records),
        Some(&mut |seq| {
            records
                .iter()
                .find_map(|record| match &record.fact {
                    NodeFact::ArtifactProduced(artifact) if record.seq == seq => {
                        Some(artifact.value.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| format!("artifact value is missing for fact {seq}"))
        }),
    )
}

fn subtree_read_rules(
    root: &TreeRootFact,
    records: &[NodeFactRecord],
    terminal: Option<&NodeFactRecord>,
) -> Result<HashMap<String, NodeReadRules>, String> {
    read_projection(root, records, terminal).map(|projection| projection.0)
}

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
    let projection = read_projection(root, records, None).unwrap();
    let read = projection.0;
    let folded = super::super::fact_replay::fold_execution_tree("tree", records)
        .unwrap()
        .unwrap();
    assert_eq!(
        projection.1.timestamp_ms as f64 / 1000.0,
        folded.aggregate.updated_at
    );
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
    for (fanout, stop_first) in [(false, false), (true, false), (false, true), (true, true)] {
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
        let mut facts = vec![
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
        ];
        if stop_first {
            facts.rotate_right(1);
        }
        for fact in facts {
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

#[test]
fn test_会話詳細の合成参照とstop先着_実行側と同じ承認待ちを導出する() {
    // Given
    for records in [
        super::super::test_helpers::delegated_routing_facts(true),
        super::super::test_helpers::delegated_routing_facts(false),
        super::super::test_helpers::composite_dynamic_fanout_facts(),
        super::super::test_helpers::composite_routing_facts(false, true),
        super::super::test_helpers::composite_routing_facts(true, true),
        super::super::test_helpers::composite_routing_facts(false, false),
        super::super::test_helpers::composite_routing_facts(true, false),
        super::super::test_helpers::stop_first_delegate_facts(),
    ] {
        let NodeFact::Started(started) = &records[0].fact else {
            panic!("root")
        };
        let root = started.root.as_deref().unwrap();
        let retry = records.iter().any(|record| record.meta.attempt == 2);
        let target = records
            .iter()
            .find(|record| {
                if retry {
                    record.meta.attempt == 2
                } else {
                    record.meta.node_name == "inner"
                }
            })
            .unwrap()
            .meta
            .node_execution_id
            .as_str();
        for end in [records.len() - 3, records.len()] {
            let records = &records[..end];
            let selected = selected_artifact_records(root, records);
            // When
            let read = subtree_read_rules(root, &selected, None).unwrap();
            let execution =
                super::super::fact_replay::fold_execution_tree(&records[0].meta.tree_id, records)
                    .unwrap()
                    .unwrap();
            // Then
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
        }
        // When
        let read =
            subtree_read_rules(root, &selected_artifact_records(root, &records), None).unwrap();
        // Then
        assert_eq!(read[target].status, NodeExecutionStatus::WaitingApproval);
    }
}

#[test]
fn test_会話詳細の承認_承認待ちだけを完了し稼働中と終端では変えない() {
    // Given
    for launched_as in [ExecutionTreeLaunch::Workflow, ExecutionTreeLaunch::Session] {
        let mut root = root(NodeDefinition {
            name: "session".into(),
            completion: NodeCompletion::require_approval(),
            ..Default::default()
        });
        root.launched_as = launched_as;
        let mut records = vec![record(
            1,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(root.clone())),
                parent: None,
                worktree: None,
            }),
        )];
        for (fact, expected) in [
            (
                NodeFact::ApprovalGranted(crate::domain::workflow::ApprovalGrantedFact {
                    comment: None,
                }),
                NodeExecutionStatus::Running,
            ),
            (
                NodeFact::StopReceived(StopReceivedFact {
                    result_summary: None,
                    token_usage: None,
                }),
                NodeExecutionStatus::Running,
            ),
            (
                NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
                NodeExecutionStatus::WaitingApproval,
            ),
            (
                NodeFact::ApprovalGranted(crate::domain::workflow::ApprovalGrantedFact {
                    comment: None,
                }),
                NodeExecutionStatus::Succeeded,
            ),
            (
                NodeFact::ApprovalGranted(crate::domain::workflow::ApprovalGrantedFact {
                    comment: None,
                }),
                NodeExecutionStatus::Succeeded,
            ),
            (NodeFact::RetryRequested, NodeExecutionStatus::Succeeded),
        ] {
            // When
            records.push(record(records.len() as i64 + 1, fact));
            // Then
            assert_replay(&root, &records, "node", expected);
        }
    }
}

#[test]
fn test_会話詳細の順序合成_children外への遷移後も実行側と同じ完了を返す() {
    // Given
    use crate::domain::workflow::{
        ArtifactProducedFact, ChildEntry, ExecutionParentRef, SequenceSpec,
    };
    let mut first = ChildEntry::reference("first");
    first.rules = Some(vec![Rule::Next("target_only".into())]);
    let mut root = root(NodeDefinition {
        name: "scope".into(),
        kind: NodeKind::Sequence(SequenceSpec {
            entry: None,
            children: vec![first],
        }),
        ..Default::default()
    });
    root.definition.as_mut().unwrap().nodes.extend([
        NodeDefinition {
            name: "first".into(),
            ..Default::default()
        },
        NodeDefinition {
            name: "target_only".into(),
            ..Default::default()
        },
    ]);
    let mut records = vec![child_record(
        1,
        "scope",
        "scope",
        NodeKindName::Sequence,
        None,
        NodeFact::Started(StartedFact {
            root: Some(Box::new(root.clone())),
            parent: None,
            worktree: None,
        }),
    )];
    for name in ["first", "target_only"] {
        let parent = ExecutionParentRef::sequence_child("scope");
        for fact in [
            NodeFact::Started(StartedFact {
                root: None,
                parent: Some(parent.clone()),
                worktree: None,
            }),
            NodeFact::SubmitReceived(SubmitReceivedFact { request_id: None }),
            NodeFact::ArtifactProduced(ArtifactProducedFact {
                value: serde_json::json!({"done": true}),
                contract: None,
                request_id: None,
            }),
            NodeFact::StopReceived(StopReceivedFact {
                result_summary: None,
                token_usage: None,
            }),
        ] {
            records.push(child_record(
                records.len() as i64 + 1,
                name,
                name,
                NodeKindName::Session,
                Some(parent.clone()),
                fact,
            ));
        }
    }
    // When / Then
    assert_replay(&root, &records, "scope", NodeExecutionStatus::Succeeded);
    // When
    let folded = super::super::fact_replay::fold_execution_tree("tree", &records)
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        folded.aggregate.node_executions[0].artifact,
        Some(serde_json::json!({"first": {"done": true}}))
    );
}

#[test]
fn test_会話の更新時刻_未受理の終了と承認と同じ成果で進まず受理した操作で進む() {
    // Given
    let records = super::super::test_helpers::session_timestamp_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root")
    };
    let root = started.root.as_deref().unwrap();
    for end in 1..=records.len() {
        let records = &records[..end];
        // When
        let projection = read_projection(root, records, None).unwrap();
        let folded =
            super::super::fact_replay::fold_execution_tree(&records[0].meta.tree_id, records)
                .unwrap()
                .unwrap();
        // Then
        assert_eq!(
            projection.1.timestamp_ms as f64 / 1000.0,
            folded.aggregate.updated_at,
            "prefix {end}"
        );
    }
}

#[test]
fn test_会話の更新時刻_成果の変更情報が欠けた場合は推測しない() {
    // Given
    let records = super::super::test_helpers::session_timestamp_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root")
    };
    // When
    let error = super::subtree_read_projection(
        started.root.as_deref().unwrap(),
        &records,
        None,
        &HashMap::new(),
        Some(&mut |seq| Err(format!("artifact value is missing for fact {seq}"))),
    )
    .err()
    .unwrap();
    // Then
    assert_eq!(error, "artifact value is missing for fact 5");
}

#[test]
fn test_定義なし会話の承認_再試行前の受理済み承認事実も実行側と同じ状態へ反映する() {
    // Given
    let mut root = root(NodeDefinition::default());
    root.definition = None;
    root.launched_as = ExecutionTreeLaunch::Session;
    let mut records = vec![
        record(
            1,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(root.clone())),
                parent: None,
                worktree: None,
            }),
        ),
        record(2, NodeFact::RetryRequested),
    ];
    let mut next = record(
        3,
        NodeFact::Started(StartedFact {
            root: None,
            parent: None,
            worktree: None,
        }),
    );
    next.meta.node_execution_id = "next".into();
    next.meta.attempt = 2;
    records.push(next);
    // When
    records.push(record(
        4,
        NodeFact::ApprovalGranted(crate::domain::workflow::ApprovalGrantedFact { comment: None }),
    ));
    let mut execution = crate::domain::workflow::entities::workflow_execution::ExecutionTree::restore_without_definition("tree", &root, 1.0);
    for record in &records {
        super::super::fact_replay::apply_record(&mut execution, record, false).unwrap();
    }
    let read = read_projection(&root, &records, None).unwrap();
    // Then
    for (id, expected) in [
        ("node", NodeExecutionStatus::Succeeded),
        ("next", NodeExecutionStatus::Running),
    ] {
        let actual = execution.node_execution(id).unwrap();
        assert_eq!(read.0[id].status, expected);
        assert_eq!(read.0[id].status, actual.status);
        assert_eq!(read.0[id].completed_at, actual.completed_at);
    }
    assert_eq!(read.1.timestamp_ms as f64 / 1000.0, execution.updated_at);
}

#[test]
fn test_会話時刻の差分投影_同じ成果と変更と委任の受理を実行側と一致させ本文を保持しない() {
    // Given
    for records in [
        super::super::test_helpers::session_timestamp_facts(),
        super::super::test_helpers::isolated_delegate_facts(false),
        super::super::test_helpers::isolated_delegate_facts(true),
        super::super::test_helpers::stop_first_delegate_facts(),
        super::super::test_helpers::isolated_session_timestamp_facts(),
        super::super::test_helpers::sibling_sequence_timestamp_facts(),
    ] {
        let NodeFact::Started(started) = &records[0].fact else {
            panic!("root")
        };
        let root = started.root.as_deref().unwrap();
        let mut state = NodeReadState::default();
        for end in 1..=records.len() {
            let selected = selected_artifact_records(root, &records[state.seq as usize..end]);
            // When
            let applied = update_read_state(
                &mut state,
                root,
                &selected,
                &super::super::history::retry_predecessors(&records[..end]),
                &mut |seq| {
                    records
                        .iter()
                        .find_map(|record| match &record.fact {
                            NodeFact::ArtifactProduced(artifact) if record.seq == seq => {
                                Some(artifact.value.clone())
                            }
                            _ => None,
                        })
                        .ok_or_else(|| "missing artifact".into())
                },
            )
            .unwrap();
            if records.get(end).is_some_and(|next| {
                super::super::fact_replay::is_submitted_artifact_pair(&records[end - 1], next)
            }) {
                assert_eq!(state.seq, records[end - 1].seq);
                continue;
            }
            let folded = super::super::fact_replay::fold_execution_tree(
                &records[0].meta.tree_id,
                &records[..end],
            )
            .unwrap()
            .unwrap();
            // Then
            assert_eq!(
                applied.timestamp_ms as f64 / 1000.0,
                folded.aggregate.updated_at,
                "prefix {end}"
            );
            assert!(state
                .artifacts
                .values()
                .all(|value| !value.to_string().contains("private")));
            assert_eq!(state.seq, records[end - 1].seq);
        }
    }
}

#[test]
fn test_時刻プレビュー_伝播が触るnodeだけを退避し成功時も失敗時も元の判定状態を保つ() {
    // Given
    let records = super::super::test_helpers::sibling_sequence_timestamp_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root")
    };
    let root = started.root.as_deref().unwrap();
    let records = &records[..records.len() - 1];
    let retries = super::super::history::retry_predecessors(records);
    let selected = selected_artifact_records(root, records);
    let mut state = NodeReadState::default();
    update_read_state(&mut state, root, &selected, &retries, &mut |seq| {
        records
            .iter()
            .find_map(|record| match &record.fact {
                NodeFact::ArtifactProduced(artifact) if record.seq == seq => {
                    Some(artifact.value.clone())
                }
                _ => None,
            })
            .ok_or_else(|| "missing artifact".into())
    })
    .unwrap();
    let last = state.pending.clone().unwrap();
    let mut projection = NodeReadProjection::new(root, root.definition.as_ref(), &retries, None);
    projection.state = state;
    let sequence = projection.state.sequences["node-side"].clone();
    let previous = projection.state.applied.timestamp_ms;
    let mut journal = CompletionJournal::new(projection.state.applied);
    // When
    projection
        .propagate_completion(&last, None, Some(&mut journal))
        .unwrap();
    // Then
    assert_eq!(
        journal
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["node-side-leaf", "node-side"]
    );
    assert!(projection.state.nodes.len() > journal.entries.len());
    assert_eq!(
        projection.state.nodes["node-side"].status,
        NodeExecutionStatus::WaitingApproval
    );
    // When
    journal.restore(&mut projection.state);
    // Then
    assert_eq!(
        projection.state.nodes["node-side"].status,
        NodeExecutionStatus::Running
    );
    assert_eq!(projection.state.sequences["node-side"], sequence);
    assert!(!projection.state.propagated.contains("node-side-leaf"));
    assert_eq!(projection.state.applied.timestamp_ms, previous);
    // Given
    let mut invalid = root.clone();
    let NodeKind::Sequence(side) = &mut invalid
        .definition
        .as_mut()
        .unwrap()
        .nodes
        .iter_mut()
        .find(|node| node.name == "side")
        .unwrap()
        .kind
    else {
        panic!("sequence")
    };
    side.children[0].rules = Some(vec![Rule::Switch {
        on: "missing".into(),
        cases: Default::default(),
        next: None,
    }]);
    projection.saved_definition = invalid.definition.as_ref();
    // When
    let failed = projection.preview_completion(&last);
    // Then
    assert!(failed.is_err());
    assert_eq!(
        projection.state.nodes["node-side"].status,
        NodeExecutionStatus::Running
    );
    assert_eq!(projection.state.sequences["node-side"], sequence);
    assert!(!projection.state.propagated.contains("node-side-leaf"));
    assert_eq!(projection.state.applied.timestamp_ms, previous);
    // Given
    let mut missing = last.clone();
    missing.meta.node_execution_id = "missing".into();
    let mut journal = CompletionJournal::new(projection.state.applied);
    // When
    projection
        .propagate_completion(&missing, None, Some(&mut journal))
        .unwrap();
    // Then
    assert!(journal.entries.is_empty());
}

#[test]
fn test_事実再生_全件と任意の分割位置の差分で完了と時刻が一致する() {
    // Given
    for records in [
        super::super::test_helpers::isolated_delegate_facts(false),
        super::super::test_helpers::isolated_delegate_facts(true),
        super::super::test_helpers::stop_first_delegate_facts(),
        super::super::test_helpers::delegated_routing_facts(true),
        super::super::test_helpers::sibling_sequence_timestamp_facts(),
    ] {
        let NodeFact::Started(started) = &records[0].fact else {
            panic!("root")
        };
        let root = started.root.as_deref().unwrap();
        let retries = super::super::history::retry_predecessors(&records);
        let selected = selected_artifact_records(root, &records);
        let expected = subtree_read_projection(root, &selected, None, &retries, None).unwrap();
        for split in 1..selected.len() {
            let mut projection =
                NodeReadProjection::new(root, root.definition.as_ref(), &retries, None);
            // When
            projection.drive(&selected[..split], None, true).unwrap();
            projection.drive(&selected[split..], None, false).unwrap();
            let actual = projection.finish(None).unwrap();
            // Then
            assert_eq!(actual.0, expected.0, "split {split}");
            assert_eq!(
                actual.1.timestamp_ms, expected.1.timestamp_ms,
                "split {split}"
            );
        }
    }
}

#[test]
fn test_定義再生の回復_終端と定義の有無により一度だけ全件再生を要求する() {
    // Given
    let records = super::super::test_helpers::sibling_sequence_timestamp_facts();
    let NodeFact::Started(started) = &records[0].fact else {
        panic!("root")
    };
    let root = started.root.as_deref().unwrap();
    let mut terminal = records.last().unwrap().clone();
    terminal.fact = NodeFact::ExecutionCompleted;
    for (root, terminal, expected) in [
        (root.clone(), None, false),
        (root.without_definition(), Some(&terminal), false),
        (root.clone(), Some(&terminal), true),
    ] {
        let mut state = NodeReadState {
            seq: 3,
            ..Default::default()
        };
        // When
        let replay = state.request_replay(&root, terminal);
        // Then
        assert_eq!(replay, expected);
        assert_eq!(state.seq, if expected { 0 } else { 3 });
        assert!(!state.request_replay(&root, terminal));
    }
}

#[test]
fn test_隔離委任子の完了_成果なしでもworktreeを精算しプレビュー後は復元する() {
    // Given
    for submitted in [false, true] {
        let records = super::super::test_helpers::isolated_delegate_facts(submitted);
        let NodeFact::Started(started) = &records[0].fact else {
            panic!("root")
        };
        let root = started.root.as_deref().unwrap();
        let selected = selected_artifact_records(root, &records);
        let retries = HashMap::new();
        let mut projection =
            NodeReadProjection::new(root, root.definition.as_ref(), &retries, None);
        let last = selected.last().unwrap();
        let child = last.meta.node_execution_id.as_str();
        projection
            .drive(&selected[..selected.len() - 1], None, false)
            .unwrap();
        let before = projection.state.artifacts.get(child).cloned();
        let rules = projection.state.nodes[child].clone();
        assert_eq!(rules.status, NodeExecutionStatus::Running);
        assert_eq!(rules.has_artifact, submitted);
        let NodeFact::Started(started) = &records
            .iter()
            .find(|record| {
                record.meta.node_execution_id == child
                    && matches!(record.fact, NodeFact::Started(_))
            })
            .unwrap()
            .fact
        else {
            panic!("child")
        };
        let worktree = started.worktree.as_ref().unwrap();
        let completed = last.clone();
        projection.apply_record(&completed).unwrap();
        let rules = projection.state.nodes[child].clone();
        let mut journal = CompletionJournal::new(projection.state.applied);
        // When
        projection
            .propagate_completion(&completed, None, Some(&mut journal))
            .unwrap();
        // Then
        assert_eq!(
            projection.state.nodes[child].status,
            NodeExecutionStatus::Succeeded
        );
        assert!(projection.state.nodes[child].has_artifact);
        assert_eq!(
            projection.state.artifacts[child]["worktree"]["path"],
            worktree.path
        );
        if submitted {
            assert_eq!(projection.state.artifacts[child]["passed"], true);
        }
        let parent = records[0].meta.node_execution_id.as_str();
        assert_eq!(
            projection.state.nodes[parent].status,
            NodeExecutionStatus::Running
        );
        assert_eq!(
            projection.state.artifacts[parent]["child"],
            projection.state.artifacts[child]
        );
        // When
        journal.restore(&mut projection.state);
        // Then
        assert_eq!(projection.state.artifacts.get(child), before.as_ref());
        assert_eq!(projection.state.nodes[child], rules);
        assert_eq!(
            projection.state.artifacts[parent]["child"],
            serde_json::Value::Null
        );
        // When
        projection
            .propagate_completion(&completed, None, None)
            .unwrap();
        let read = projection.finish(None).unwrap();
        let execution =
            super::super::fact_replay::fold_execution_tree(&records[0].meta.tree_id, &records)
                .unwrap()
                .unwrap();
        // Then
        for node in &execution.aggregate.node_executions {
            assert_eq!(read.0[&node.id].status, node.status);
            assert_eq!(read.0[&node.id].completed_at, node.completed_at);
        }
    }
}
