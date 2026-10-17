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
