use super::*;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    FacetRefs, FanoutSpec, ItemsSource, NodeCompletion, NodeDefinition, NodeKind, SequenceSpec,
    SessionSpec,
};
use crate::domain::workspace_tree::WorkspaceTreeVisibilityPolicy;

fn definition() -> WorkflowDefinition {
    WorkflowDefinition {
        name: "review".to_string(),
        description: String::new(),
        builtin: false,
        schemas: BTreeMap::new(),
        nodes: vec![NodeDefinition {
            name: "plan".to_string(),
            kind: NodeKind::Session(SessionSpec {
                provider: ProviderKind::Claude,
                model: None,
                permission: None,
                facets: FacetRefs::default(),
            }),
            artifact: None,
            input: Vec::new(),
            completion: NodeCompletion::Auto,
            worktree: None,
        }],
        entry: "plan".to_string(),
    }
}

fn parent_ref(kind: NodeKindName, parent_id: &str, child_index: usize) -> ExecutionParentRef {
    if kind == NodeKindName::Fanout {
        ExecutionParentRef::fanout_child(parent_id, None, child_index)
    } else {
        ExecutionParentRef::sequence_child(parent_id)
    }
}

fn status_fact(
    execution_id: &str,
    node_execution_id: &str,
    status: WorkspaceNodeStatus,
    timestamp: f64,
) -> Option<WorkspaceStructureFact> {
    match status {
        WorkspaceNodeStatus::Running => None,
        WorkspaceNodeStatus::Completed => Some(WorkspaceStructureFact::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            timestamp,
        }),
        WorkspaceNodeStatus::Waiting => Some(WorkspaceStructureFact::NodeApprovalRequested {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            timestamp,
        }),
        WorkspaceNodeStatus::Failed | WorkspaceNodeStatus::Aborted => {
            Some(WorkspaceStructureFact::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                reason: "test failure".to_string(),
                failure_kind: if status == WorkspaceNodeStatus::Aborted {
                    NodeExecutionFailureKind::UserAbort
                } else {
                    NodeExecutionFailureKind::ValidationFailure
                },
                timestamp,
            })
        }
        WorkspaceNodeStatus::Paused => None,
    }
}

fn branch_classification(
    kind: NodeKindName,
    parent_status: WorkspaceNodeStatus,
    child_statuses: &[WorkspaceNodeStatus],
) -> WorkspaceNodeStatusClassification {
    let execution_id = "00000000-0000-4000-8000-0000000000d1";
    let mut facts = vec![
        WorkspaceStructureFact::WorkflowStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "classification".to_string(),
            worktree_path: "/repo".to_string(),
            definition: definition(),
            timestamp: 1.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "branch".to_string(),
            node_name: "branch".to_string(),
            kind,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
    ];
    for (index, status) in child_statuses.iter().copied().enumerate() {
        let node_execution_id = format!("child-{index}");
        facts.push(WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.clone(),
            node_name: node_execution_id.clone(),
            kind: NodeKindName::Command,
            attempt: 1,
            parent: Some(parent_ref(kind, "branch", index)),
            timestamp: 3.0 + index as f64,
        });
        if let Some(fact) = status_fact(
            execution_id,
            &node_execution_id,
            status,
            10.0 + index as f64,
        ) {
            facts.push(fact);
        }
    }
    if let Some(fact) = status_fact(execution_id, "branch", parent_status, 20.0) {
        facts.push(fact);
    }
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(&mut tree, facts).unwrap();
    tree.nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("branch"))
        .unwrap()
        .status_classification
}

#[test]
fn test_親分類集約_sequenceとfanoutが同じ重大度順を使う() {
    // Given
    let cases: [(
        WorkspaceNodeStatus,
        &[WorkspaceNodeStatus],
        WorkspaceNodeStatusClassification,
    ); 4] = [
        (
            WorkspaceNodeStatus::Completed,
            &[WorkspaceNodeStatus::Completed],
            WorkspaceNodeStatusClassification::Idle,
        ),
        (
            WorkspaceNodeStatus::Running,
            &[WorkspaceNodeStatus::Completed],
            WorkspaceNodeStatusClassification::Active,
        ),
        (
            WorkspaceNodeStatus::Completed,
            &[WorkspaceNodeStatus::Running, WorkspaceNodeStatus::Waiting],
            WorkspaceNodeStatusClassification::Attention,
        ),
        (
            WorkspaceNodeStatus::Completed,
            &[
                WorkspaceNodeStatus::Running,
                WorkspaceNodeStatus::Waiting,
                WorkspaceNodeStatus::Failed,
            ],
            WorkspaceNodeStatusClassification::Failure,
        ),
    ];

    // When / Then
    for (parent_status, child_statuses, expected) in cases {
        for kind in [NodeKindName::Sequence, NodeKindName::Fanout] {
            assert_eq!(
                branch_classification(kind, parent_status, child_statuses),
                expected,
                "{kind:?}"
            );
        }
    }
}

#[test]
fn test_親分類集約_子孫のfailureをsequenceとfanoutの祖先まで反映する() {
    // Given
    let execution_id = "00000000-0000-4000-8000-0000000000d2";
    let mut tree = WorkspaceTree::empty("/repo");
    let facts = [
        WorkspaceStructureFact::WorkflowStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "classification".to_string(),
            worktree_path: "/repo".to_string(),
            definition: definition(),
            timestamp: 1.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "outer-sequence".to_string(),
            node_name: "outer".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "inner-fanout".to_string(),
            node_name: "inner".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child("outer-sequence")),
            timestamp: 3.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "failed-leaf".to_string(),
            node_name: "leaf".to_string(),
            kind: NodeKindName::Command,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child("inner-fanout", None, 0)),
            timestamp: 4.0,
        },
        WorkspaceStructureFact::NodeFailed {
            execution_id: execution_id.to_string(),
            node_execution_id: "failed-leaf".to_string(),
            reason: "test failure".to_string(),
            failure_kind: NodeExecutionFailureKind::ValidationFailure,
            timestamp: 5.0,
        },
        WorkspaceStructureFact::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: "inner-fanout".to_string(),
            timestamp: 6.0,
        },
        WorkspaceStructureFact::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_execution_id: "outer-sequence".to_string(),
            timestamp: 7.0,
        },
    ];

    // When
    WorkspaceTreeProjector::project(&mut tree, facts).unwrap();

    // Then
    for node_execution_id in ["inner-fanout", "outer-sequence"] {
        assert_eq!(
            tree.nodes()
                .iter()
                .find(|node| { node.node_execution_id.as_deref() == Some(node_execution_id) })
                .unwrap()
                .status_classification,
            WorkspaceNodeStatusClassification::Failure
        );
    }
}

#[test]
fn root_sequence_tree_nodes_get_distinct_ids_per_execution() {
    let mut tree = WorkspaceTree::empty("/repo");
    for (execution_id, node_execution_id) in [
        ("00000000-0000-4000-8000-00000000000a", "seq-a"),
        ("00000000-0000-4000-8000-00000000000b", "seq-b"),
    ] {
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: "main".to_string(),
                    kind: NodeKindName::Sequence,
                    attempt: 1,
                    parent: None,
                    timestamp: 2.0,
                },
            ],
        )
        .unwrap();
    }

    let ids: Vec<&str> = tree
        .nodes()
        .iter()
        .filter(|node| node.kind == WorkspaceNodeKind::Sequence)
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(
        ids[0], ids[1],
        "root sequence tree node ids must not collide across executions"
    );
}

#[test]
fn sequence_branch_propagates_child_updated_at_to_workflow_root() {
    let execution_id = "00000000-0000-4000-8000-000000000001";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "seq-main".to_string(),
                node_name: "main".to_string(),
                kind: NodeKindName::Sequence,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "leaf-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::sequence_child("seq-main")),
                timestamp: 9.0,
            },
        ],
    )
    .unwrap();

    let sequence = tree
        .nodes()
        .iter()
        .find(|node| node.kind == WorkspaceNodeKind::Sequence)
        .unwrap();
    assert_eq!(sequence.status, WorkspaceNodeStatus::Running);
    assert_eq!(sequence.updated_at_bits, 9.0f64.to_bits());
    let workflow = tree.workflow_node(execution_id).unwrap();
    assert_eq!(
        workflow.updated_at_bits,
        9.0f64.to_bits(),
        "a leaf update inside a sequence branch must reach the workflow root"
    );
}

#[test]
fn workspace_tree_projector_owns_parentage_identity_and_occurrence_order() {
    let execution_id = "00000000-0000-4000-8000-000000000001";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 0,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-2".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 3.0,
            },
        ],
    )
    .unwrap();

    let leaves = tree
        .nodes()
        .iter()
        .filter(|node| node.kind == WorkspaceNodeKind::WorkflowSession)
        .collect::<Vec<_>>();
    assert_eq!(leaves.len(), 2);
    assert_ne!(leaves[0].id, leaves[1].id);
    assert_eq!(leaves[0].parent_id.as_deref(), Some(execution_id));
    assert!(leaves[0].sibling_order < leaves[1].sibling_order);
}

#[test]
fn workflow_without_started_nodes_has_an_empty_branch_and_no_preferred_node() {
    let execution_id = "00000000-0000-4000-8000-000000000099";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::WorkflowStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            definition: definition(),
            timestamp: 1.0,
        }],
    )
    .unwrap();

    let public_nodes = tree
        .nodes()
        .iter()
        .filter(|node| !node.is_internal_rule_record())
        .collect::<Vec<_>>();
    assert_eq!(public_nodes.len(), 1);
    assert_eq!(public_nodes[0].kind, WorkspaceNodeKind::Workflow);
    assert_eq!(tree.preferred_node_id(&HashSet::new()), None);
}

#[test]
fn workspace_tree_rejects_duplicate_session_binding() {
    let execution_id = "00000000-0000-4000-8000-0000000000c1";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-a".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-b".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
                parent: None,
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-a".to_string(),
                session_id: "session".to_string(),
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();

    assert!(matches!(
        WorkspaceTreeProjector::project(
            &mut tree,
            [WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-b".to_string(),
                session_id: "session".to_string(),
                timestamp: 5.0,
            }],
        ),
        Err(WorkspaceTreeError::DuplicateSession(_))
    ));
}

fn two_execution_session_tree() -> WorkspaceTree {
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: "00000000-0000-4000-8000-0000000000a1".to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: "00000000-0000-4000-8000-0000000000a1".to_string(),
                node_execution_id: "node-a".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: "00000000-0000-4000-8000-0000000000b1".to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: "00000000-0000-4000-8000-0000000000b1".to_string(),
                node_execution_id: "node-b".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();
    tree
}

fn workflow_session_binding(
    session_id: &str,
    execution_id: &str,
    node_execution_id: &str,
    updated_at: f64,
) -> WorkspaceStructureFact {
    WorkspaceStructureFact::NodeAgentBound {
        execution_id: execution_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        session_id: session_id.to_string(),
        timestamp: updated_at,
    }
}

#[test]
fn workflow_session_binding_rejects_an_execution_and_node_from_different_runs() {
    let mut tree = two_execution_session_tree();

    let error = WorkspaceTreeProjector::project(
        &mut tree,
        [workflow_session_binding(
            "crossed-session",
            "00000000-0000-4000-8000-0000000000a1",
            "node-b",
            5.0,
        )],
    )
    .unwrap_err();

    assert!(matches!(error, WorkspaceTreeError::MissingNodeExecution(_)));
    assert!(tree
        .nodes()
        .iter()
        .all(|node| node.session_id.as_deref() != Some("crossed-session")));
}

#[test]
fn workflow_session_binding_binds_a_matching_execution_and_node_pair() {
    let mut tree = two_execution_session_tree();

    WorkspaceTreeProjector::project(
        &mut tree,
        [workflow_session_binding(
            "matched-session",
            "00000000-0000-4000-8000-0000000000b1",
            "node-b",
            5.0,
        )],
    )
    .unwrap();

    let node = tree.session_node("matched-session").unwrap();
    assert_eq!(
        node.execution_id.as_deref(),
        Some("00000000-0000-4000-8000-0000000000b1")
    );
    assert_eq!(node.node_execution_id.as_deref(), Some("node-b"));
}

#[test]
fn workflow_session_binding_rejects_duplicate_session_identity() {
    let mut tree = two_execution_session_tree();
    WorkspaceTreeProjector::project(
        &mut tree,
        [workflow_session_binding(
            "rebound-session",
            "00000000-0000-4000-8000-0000000000a1",
            "node-a",
            5.0,
        )],
    )
    .unwrap();

    let error = WorkspaceTreeProjector::project(
        &mut tree,
        [workflow_session_binding(
            "rebound-session",
            "00000000-0000-4000-8000-0000000000b1",
            "node-b",
            6.0,
        )],
    )
    .unwrap_err();

    assert!(matches!(error, WorkspaceTreeError::DuplicateSession(_)));
    let node = tree.session_node("rebound-session").unwrap();
    assert_eq!(
        node.execution_id.as_deref(),
        Some("00000000-0000-4000-8000-0000000000a1")
    );
    assert_eq!(node.node_execution_id.as_deref(), Some("node-a"));
    assert_eq!(node.status, WorkspaceNodeStatus::Running);
    assert_eq!(node.error_reason, None);
    assert_eq!(node.updated_at_bits, 5.0f64.to_bits());
}

#[test]
fn completed_workflow_session_keeps_agent_session_binding() {
    let execution_id = "00000000-0000-4000-8000-000000000001";
    let mut workflow_tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut workflow_tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-execution".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 0,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-execution".to_string(),
                session_id: "workflow-session".to_string(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: "node-execution".to_string(),
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();
    assert!(workflow_tree
        .session_node("workflow-session")
        .is_some_and(|node| node.kind == WorkspaceNodeKind::WorkflowSession));
}

#[test]
fn archive_visibility_hides_only_workflow_branch() {
    let execution_id = "00000000-0000-4000-8000-000000000001";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::WorkflowStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            definition: definition(),
            timestamp: 1.0,
        }],
    )
    .unwrap();
    let hidden = WorkspaceTreeVisibilityPolicy::hidden_branch_ids(&tree, [execution_id]);
    assert_eq!(hidden, HashSet::from([execution_id.to_string()]));
    assert_eq!(tree.nodes().len(), 1);
}

#[test]
fn workflow_recovery_reason_is_derived_from_stable_owner_order() {
    let execution_id = "00000000-0000-4000-8000-000000000001";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-z".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 0,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-z".to_string(),
                session_id: "session-z".to_string(),
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-a".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "n-a".to_string(),
                session_id: "session-a".to_string(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::RecoveryFenceProjected {
                owner: "session-z".to_string(),
                reason: Some("session-z recovery".to_string()),
            },
            WorkspaceStructureFact::RecoveryFenceProjected {
                owner: "session-a".to_string(),
                reason: Some("session-a recovery".to_string()),
            },
            WorkspaceStructureFact::RecoveryFenceProjected {
                owner: execution_id.to_string(),
                reason: Some("execution recovery".to_string()),
            },
            WorkspaceStructureFact::WorkflowSummaryProjected {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                status: ExecutionStatus::Interrupted,
                updated_at: 5.0,
            },
        ],
    )
    .unwrap();

    let workflow = tree.workflow_node(execution_id).unwrap();
    assert_eq!(
        workflow.resume_unavailable_reason.as_deref(),
        Some("session-a recovery")
    );
    assert!(!workflow.can_resume);

    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::RecoveryFenceProjected {
                owner: "session-a".to_string(),
                reason: None,
            },
            WorkspaceStructureFact::RecoveryFenceProjected {
                owner: execution_id.to_string(),
                reason: None,
            },
        ],
    )
    .unwrap();
    assert_eq!(
        tree.workflow_node(execution_id)
            .unwrap()
            .resume_unavailable_reason
            .as_deref(),
        Some("session-z recovery")
    );
}

#[test]
fn opaque_identity_digest_matches_sha256_and_normalizes_uuid() {
    assert_eq!(
        opaque_id("node", "abc"),
        "node-ba7816bf8f01cfea414140de5dae2223"
    );
    assert_eq!(
        opaque_workflow_node_id("{00000000-0000-4000-8000-000000000001}", "abc").unwrap(),
        "node-w-00000000000040008000000000000001-ba7816bf8f01cfea414140de5dae2223"
    );
}

fn looping_fanout_facts(execution_id: &str, items: ItemsSource) -> Vec<WorkspaceStructureFact> {
    let mut workflow = definition();
    workflow.nodes.extend([
        NodeDefinition {
            name: "review-cycle".to_string(),
            kind: NodeKind::Sequence(SequenceSpec {
                children: vec![crate::domain::workflow::ChildEntry::reference("reviews")],
                ..SequenceSpec::default()
            }),
            ..NodeDefinition::default()
        },
        NodeDefinition {
            name: "reviews".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                children: vec![crate::domain::workflow::ChildEntry::reference("plan")],
                items: Some(items),
            }),
            ..NodeDefinition::default()
        },
    ]);
    vec![
        WorkspaceStructureFact::WorkflowStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            worktree_path: "/repo".to_string(),
            definition: workflow,
            timestamp: 1.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "review-cycle-1".to_string(),
            node_name: "review-cycle".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "reviews-1".to_string(),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 1,
            parent: Some(ExecutionParentRef::sequence_child("review-cycle-1")),
            timestamp: 3.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "plan-1".to_string(),
            node_name: "plan".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: Some(ExecutionParentRef::fanout_child("reviews-1", Some(0), 0)),
            timestamp: 4.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "review-cycle-2".to_string(),
            node_name: "review-cycle".to_string(),
            kind: NodeKindName::Sequence,
            attempt: 2,
            parent: None,
            timestamp: 5.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "reviews-2".to_string(),
            node_name: "reviews".to_string(),
            kind: NodeKindName::Fanout,
            attempt: 2,
            parent: Some(ExecutionParentRef::sequence_child("review-cycle-2")),
            timestamp: 6.0,
        },
        WorkspaceStructureFact::NodeStarted {
            execution_id: execution_id.to_string(),
            node_execution_id: "plan-2".to_string(),
            node_name: "plan".to_string(),
            kind: NodeKindName::Session,
            attempt: 2,
            parent: Some(ExecutionParentRef::fanout_child("reviews-2", Some(0), 0)),
            timestamp: 7.0,
        },
    ]
}

fn assert_looping_fanout_projection(tree: &WorkspaceTree) {
    let ids = tree
        .nodes()
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), tree.nodes().len());

    let execution_node = |node_execution_id: &str| {
        tree.nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id))
            .unwrap()
    };
    let first_fanout = execution_node("reviews-1");
    let second_fanout = execution_node("reviews-2");
    let first_child = execution_node("plan-1");
    let second_child = execution_node("plan-2");

    assert_ne!(first_child.id, second_child.id);
    assert_eq!(
        first_child.parent_id.as_deref(),
        Some(first_fanout.id.as_str())
    );
    assert_eq!(
        second_child.parent_id.as_deref(),
        Some(second_fanout.id.as_str())
    );
    assert_eq!(
        tree.nodes()
            .iter()
            .filter(|node| {
                node.parent_id.as_deref() == Some(first_fanout.id.as_str())
                    || node.parent_id.as_deref() == Some(second_fanout.id.as_str())
            })
            .count(),
        2
    );
}

#[test]
fn test_fanout子key_親node_executionと展開座標と出現回数を単一形式にする() {
    // Given / When
    let static_key = fanout_child_occurrence_key("fanout-execution", Some(2), 1, "child", 3);
    let dynamic_key = fanout_dynamic_child_occurrence_key("fanout-execution", 1, "child", 3);

    // Then
    assert_eq!(
        static_key,
        "fanout-child\0fanout-execution\0Some(2)\01\0child\03"
    );
    assert_eq!(
        dynamic_key,
        "fanout-child\0fanout-execution\0None\01\0child\03"
    );
}

#[test]
fn test_静的fanout子projection_後方辺の周回ごとに独立して再現可能になる() {
    // Given
    let execution_id = "00000000-0000-4000-8000-000000000154";
    let facts = looping_fanout_facts(
        execution_id,
        ItemsSource::Literal(vec![serde_json::json!("only")]),
    );

    // When
    let mut first = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(&mut first, facts.clone()).unwrap();
    let mut second = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(&mut second, facts).unwrap();

    // Then
    assert_looping_fanout_projection(&first);
    assert_eq!(first, second);
    assert_eq!(
        first
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("plan-1"))
            .map(|node| node.id.as_str()),
        second
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("plan-1"))
            .map(|node| node.id.as_str())
    );
}

#[test]
fn test_動的fanout子projection_後方辺の周回ごとに独立する() {
    // Given
    let execution_id = "00000000-0000-4000-8000-000000000155";
    let facts = looping_fanout_facts(
        execution_id,
        ItemsSource::ArtifactField {
            node: "source".to_string(),
            field_path: crate::domain::workflow::FieldPath::new(["items"]),
        },
    );

    // When
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(&mut tree, facts).unwrap();

    // Then
    assert_looping_fanout_projection(&tree);
    for (parent_node_execution_id, child_node_execution_id) in
        [("reviews-1", "plan-1"), ("reviews-2", "plan-2")]
    {
        let expected_id = opaque_workflow_node_id(
            execution_id,
            &fanout_dynamic_child_occurrence_key(parent_node_execution_id, 0, "plan", 0),
        )
        .unwrap();
        assert_eq!(
            tree.nodes()
                .iter()
                .find(|node| { node.node_execution_id.as_deref() == Some(child_node_execution_id) })
                .map(|node| node.id.as_str()),
            Some(expected_id.as_str())
        );
    }
}

#[test]
fn literal_fanout_projects_only_started_children_in_event_order() {
    let execution_id = "00000000-0000-4000-8000-000000000149";
    let mut definition = definition();
    definition.nodes.push(NodeDefinition {
        name: "fanout".to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![
                crate::domain::workflow::ChildEntry::reference("child-a"),
                crate::domain::workflow::ChildEntry::reference("child-b"),
            ],
            items: Some(ItemsSource::Literal(vec![serde_json::json!("only")])),
        }),
        ..NodeDefinition::default()
    });
    let mut tree = WorkspaceTree::empty("/repo");

    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition,
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-execution".to_string(),
                node_name: "fanout".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-b-execution".to_string(),
                node_name: "child-b".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child(
                    "fanout-execution",
                    Some(0),
                    1,
                )),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "child-a-execution".to_string(),
                node_name: "child-a".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child(
                    "fanout-execution",
                    Some(0),
                    0,
                )),
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();

    let fanout = tree
        .nodes()
        .iter()
        .find(|node| node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record())
        .unwrap();
    let mut children = tree
        .nodes()
        .iter()
        .filter(|node| node.parent_id.as_deref() == Some(fanout.id.as_str()))
        .collect::<Vec<_>>();
    children.sort_by_key(|node| node.sibling_order);
    assert_eq!(
        children
            .iter()
            .map(|node| node.title.as_str())
            .collect::<Vec<_>>(),
        vec!["child-b", "child-a"]
    );
}

#[test]
fn artifact_item_fanout_without_started_children_has_an_empty_branch() {
    let execution_id = "00000000-0000-4000-8000-000000000151";
    let mut workflow = definition();
    workflow.nodes.push(NodeDefinition {
        name: "matrix".to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![crate::domain::workflow::ChildEntry::reference("plan")],
            items: Some(ItemsSource::ArtifactField {
                node: "source".to_string(),
                field_path: crate::domain::workflow::FieldPath::new(["items"]),
            }),
        }),
        ..NodeDefinition::default()
    });
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "matrix-parent".to_string(),
                node_name: "matrix".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
        ],
    )
    .unwrap();

    let fanout = tree
        .nodes()
        .iter()
        .find(|node| node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record())
        .unwrap();
    let fanout_id = fanout.id.clone();
    assert!(!tree
        .nodes()
        .iter()
        .any(|node| node.parent_id.as_deref() == Some(fanout_id.as_str())));

    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "matrix-child-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child(
                    "matrix-parent",
                    Some(0),
                    0,
                )),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "matrix-child-2".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child(
                    "matrix-parent",
                    Some(1),
                    0,
                )),
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();
    let children = tree
        .nodes()
        .iter()
        .filter(|node| node.parent_id.as_deref() == Some(fanout_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 2);
    assert_ne!(children[0].id, children[1].id);
}

#[test]
fn fanout_occurrences_are_distinct_and_children_stay_nested_in_event_order() {
    let execution_id = "00000000-0000-4000-8000-000000000152";
    let mut workflow = definition();
    workflow.nodes.push(NodeDefinition {
        name: "reviews".to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![crate::domain::workflow::ChildEntry::reference("plan")],
            items: None,
        }),
        ..NodeDefinition::default()
    });
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-1".to_string(),
                node_name: "reviews".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-1-child-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child("fanout-1", None, 0)),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-1-child-2".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
                parent: Some(ExecutionParentRef::fanout_child("fanout-1", None, 0)),
                timestamp: 4.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-2".to_string(),
                node_name: "reviews".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 2,
                parent: None,
                timestamp: 5.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "fanout-2-child".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 3,
                parent: Some(ExecutionParentRef::fanout_child("fanout-2", None, 0)),
                timestamp: 6.0,
            },
        ],
    )
    .unwrap();

    let mut fanouts = tree
        .nodes()
        .iter()
        .filter(|node| node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record())
        .collect::<Vec<_>>();
    fanouts.sort_by_key(|node| node.sibling_order);
    assert_eq!(fanouts.len(), 2);
    assert_ne!(fanouts[0].id, fanouts[1].id);
    let children = |parent_id: &str| {
        tree.nodes()
            .iter()
            .filter(|node| node.parent_id.as_deref() == Some(parent_id))
            .collect::<Vec<_>>()
    };
    let first_children = children(&fanouts[0].id);
    let second_children = children(&fanouts[1].id);
    assert_eq!(first_children.len(), 2);
    assert_eq!(second_children.len(), 1);
    assert_ne!(first_children[0].id, first_children[1].id);
}

#[test]
fn branch_status_capabilities_and_session_activity_are_backend_aggregated() {
    let execution_id = "00000000-0000-4000-8000-000000000153";
    let mut workflow = definition();
    workflow.nodes.extend([
        NodeDefinition {
            name: "checks".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                children: vec![
                    crate::domain::workflow::ChildEntry::reference("lint"),
                    crate::domain::workflow::ChildEntry::reference("test"),
                ],
                items: None,
            }),
            ..NodeDefinition::default()
        },
        NodeDefinition {
            name: "lint".to_string(),
            ..NodeDefinition::default()
        },
        NodeDefinition {
            name: "test".to_string(),
            ..NodeDefinition::default()
        },
    ]);
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "plan".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "plan".to_string(),
                session_id: "plan-session".to_string(),
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: "plan".to_string(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "checks".to_string(),
                node_name: "checks".to_string(),
                kind: NodeKindName::Fanout,
                attempt: 1,
                parent: None,
                timestamp: 4.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: "checks".to_string(),
                timestamp: 5.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "lint".to_string(),
                node_name: "lint".to_string(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child("checks", None, 0)),
                timestamp: 6.0,
            },
            WorkspaceStructureFact::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: "lint".to_string(),
                reason: "internal lint failure".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                timestamp: 7.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "test".to_string(),
                node_name: "test".to_string(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: Some(ExecutionParentRef::fanout_child("checks", None, 1)),
                timestamp: 8.0,
            },
            WorkspaceStructureFact::NodeApprovalRequested {
                execution_id: execution_id.to_string(),
                node_execution_id: "test".to_string(),
                timestamp: 9.0,
            },
            WorkspaceStructureFact::WorkflowSummaryProjected {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                status: ExecutionStatus::WaitingApproval,
                updated_at: 10.0,
            },
        ],
    )
    .unwrap();

    let workflow = tree.workflow_node(execution_id).unwrap();
    assert_eq!(workflow.status, WorkspaceNodeStatus::Waiting);
    assert_eq!(
        workflow.status_classification,
        WorkspaceNodeStatusClassification::Attention
    );
    assert!(!workflow.can_stop);
    assert!(!workflow.can_resume);
    assert!(workflow.can_abort);
    assert!(!workflow.can_archive);
    assert_eq!(
        tree.session_node("plan-session").unwrap().status,
        WorkspaceNodeStatus::Completed
    );
    let fanout = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("checks"))
        .unwrap();
    assert_eq!(fanout.status, WorkspaceNodeStatus::Completed);
    assert_eq!(
        fanout.status_classification,
        WorkspaceNodeStatusClassification::Failure
    );
    let waiting = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("test"))
        .unwrap();
    assert_eq!(waiting.status, WorkspaceNodeStatus::Waiting);
    assert_eq!(
        waiting.status_classification,
        WorkspaceNodeStatusClassification::Attention
    );
    assert!(waiting.can_approve);
}

#[test]
fn terminal_workflows_hide_every_unstarted_leaf_and_branch() {
    let execution_id = "00000000-0000-4000-8000-000000000150";
    let mut workflow = definition();
    workflow.nodes.push(NodeDefinition {
        name: "dynamic".to_string(),
        kind: NodeKind::Fanout(FanoutSpec {
            children: vec![crate::domain::workflow::ChildEntry::reference("plan")],
            items: Some(ItemsSource::ArtifactField {
                node: "source".to_string(),
                field_path: crate::domain::workflow::FieldPath::new(["items"]),
            }),
        }),
        ..NodeDefinition::default()
    });
    let mut tree = WorkspaceTree::empty("/repo");

    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: workflow,
                timestamp: 1.0,
            },
            WorkspaceStructureFact::WorkflowSummaryProjected {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                status: ExecutionStatus::Completed,
                updated_at: 2.0,
            },
        ],
    )
    .unwrap();

    let public_nodes = tree
        .nodes()
        .iter()
        .filter(|node| !node.is_internal_rule_record())
        .collect::<Vec<_>>();
    assert_eq!(public_nodes.len(), 1);
    assert_eq!(public_nodes[0].kind, WorkspaceNodeKind::Workflow);
    assert_eq!(public_nodes[0].status, WorkspaceNodeStatus::Completed);
}

#[test]
fn workflow_root_order_matches_the_audit_golden() {
    let mut tree = WorkspaceTree::empty("/repo");

    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: "00000000-0000-4000-8000-000000000002".to_string(),
                workflow_name: "zeta".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
                workflow_name: "Echo".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 2.0,
            },
        ],
    )
    .unwrap();

    let mut roots = tree
        .nodes()
        .iter()
        .filter(|node| node.parent_id.is_none())
        .collect::<Vec<_>>();
    roots.sort_by_key(|node| node.sibling_order);
    assert_eq!(
        roots
            .iter()
            .map(|node| (node.id.as_str(), node.title.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("00000000-0000-4000-8000-000000000001", "Echo"),
            ("00000000-0000-4000-8000-000000000002", "zeta"),
        ]
    );
}

fn repeated_command_occurrence_tree() -> WorkspaceTree {
    let execution_id = "00000000-0000-4000-8000-000000000011";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeCommandPrepared {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-1".to_string(),
                display_command: "first command".to_string(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-1".to_string(),
                timestamp: 4.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-2".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Command,
                attempt: 2,
                parent: None,
                timestamp: 5.0,
            },
            WorkspaceStructureFact::NodeCommandPrepared {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-2".to_string(),
                display_command: "second command".to_string(),
                timestamp: 6.0,
            },
            WorkspaceStructureFact::NodeApprovalRequested {
                execution_id: execution_id.to_string(),
                node_execution_id: "occurrence-2".to_string(),
                timestamp: 7.0,
            },
        ],
    )
    .unwrap();
    tree
}

#[test]
fn each_occurrence_keeps_its_detail_and_only_waiting_occurrence_can_approve() {
    let tree = repeated_command_occurrence_tree();
    let first = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("occurrence-1"))
        .unwrap();
    let second = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("occurrence-2"))
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.display_command.as_deref(), Some("first command"));
    assert!(!first.can_approve);
    assert_eq!(second.display_command.as_deref(), Some("second command"));
    assert!(second.can_approve);
}

#[test]
fn command_detail_remains_bound_to_the_selected_occurrence() {
    let tree = repeated_command_occurrence_tree();
    let first = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("occurrence-1"))
        .unwrap();
    let second = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("occurrence-2"))
        .unwrap();

    assert_eq!(first.display_command.as_deref(), Some("first command"));
    assert_eq!(second.display_command.as_deref(), Some("second command"));
    assert_ne!(first.id, second.id);
}

#[test]
fn repeated_session_occurrences_keep_distinct_session_detail_and_lookup() {
    let execution_id = "00000000-0000-4000-8000-000000000013";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-occurrence-1".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-occurrence-1".to_string(),
                session_id: "stored-session-1".to_string(),
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-occurrence-1".to_string(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-occurrence-2".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
                parent: None,
                timestamp: 4.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-occurrence-2".to_string(),
                session_id: "stored-session-2".to_string(),
                timestamp: 4.0,
            },
        ],
    )
    .unwrap();

    let first = tree.session_node("stored-session-1").unwrap();
    let second = tree.session_node("stored-session-2").unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.session_id.as_deref(), Some("stored-session-1"));
    assert_eq!(second.session_id.as_deref(), Some("stored-session-2"));
    assert_eq!(first.status, WorkspaceNodeStatus::Completed);
    assert_eq!(second.status, WorkspaceNodeStatus::Running);
}

#[test]
fn test_session表示名_単独rootだけproviderタイトルを使いrenameを最優先する() {
    let standalone_id = "00000000-0000-4000-8000-000000000014";
    let workflow_id = "00000000-0000-4000-8000-000000000015";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: standalone_id.to_string(),
                workflow_name: "session".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: standalone_id.to_string(),
                node_execution_id: standalone_id.to_string(),
                node_name: "session".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeSessionDisplayNameProjected {
                execution_id: standalone_id.to_string(),
                node_execution_id: standalone_id.to_string(),
                manual_name: None,
                provider_session_title: Some("Provider title".to_string()),
            },
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: workflow_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: workflow_id.to_string(),
                node_execution_id: "workflow-session".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 4.0,
            },
            WorkspaceStructureFact::NodeSessionDisplayNameProjected {
                execution_id: workflow_id.to_string(),
                node_execution_id: "workflow-session".to_string(),
                manual_name: None,
                provider_session_title: Some("Hidden provider title".to_string()),
            },
        ],
    )
    .unwrap();

    let standalone = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some(standalone_id))
        .unwrap();
    let workflow_session = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("workflow-session"))
        .unwrap();
    assert!(standalone.is_standalone_session_root());
    assert_eq!(standalone.title, "Provider title");
    assert_eq!(workflow_session.title, "plan");

    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::NodeSessionDisplayNameProjected {
            execution_id: standalone_id.to_string(),
            node_execution_id: standalone_id.to_string(),
            manual_name: Some("Manual name".to_string()),
            provider_session_title: Some("Updated provider title".to_string()),
        }],
    )
    .unwrap();
    let standalone = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some(standalone_id))
        .unwrap();
    assert_eq!(standalone.title, "Manual name");
}

#[test]
fn test_session表示名_親ありで実行idとnode実行idが同じsessionは単独rootにしない() {
    let execution_id = "00000000-0000-4000-8000-000000000018";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "sequence".to_string(),
                node_name: "main".to_string(),
                kind: NodeKindName::Sequence,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: execution_id.to_string(),
                node_name: "nested-session".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::sequence_child("sequence")),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeSessionDisplayNameProjected {
                execution_id: execution_id.to_string(),
                node_execution_id: execution_id.to_string(),
                manual_name: None,
                provider_session_title: Some("Hidden provider title".to_string()),
            },
        ],
    )
    .unwrap();

    let session = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some(execution_id))
        .unwrap();
    assert!(!session.is_standalone_session_root());
    assert!(session.id.starts_with("node-w-"));
    assert_eq!(session.title, "nested-session");
}

#[test]
fn test_session表示名変更可否_session_bind後だけ真になる() {
    let execution_id = "00000000-0000-4000-8000-000000000016";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "sequence".to_string(),
                node_name: "main".to_string(),
                kind: NodeKindName::Sequence,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "session-node".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: Some(ExecutionParentRef::sequence_child("sequence")),
                timestamp: 3.0,
            },
        ],
    )
    .unwrap();
    assert!(tree.nodes().iter().all(|node| !node.can_rename));
    let unbound = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("session-node"))
        .unwrap();
    assert_eq!(
        unbound.status_classification,
        WorkspaceNodeStatusClassification::Unbound
    );

    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::NodeAgentBound {
            execution_id: execution_id.to_string(),
            node_execution_id: "session-node".to_string(),
            session_id: "agent-session".to_string(),
            timestamp: 4.0,
        }],
    )
    .unwrap();

    let session = tree.session_node("agent-session").unwrap();
    assert!(session.can_rename);
    assert_ne!(
        session.status_classification,
        WorkspaceNodeStatusClassification::Unbound
    );
    assert!(tree
        .nodes()
        .iter()
        .filter(|node| node.kind != WorkspaceNodeKind::WorkflowSession)
        .all(|node| !node.can_rename));
}

#[test]
fn test_親分類集約_bind前sessionだけならunboundで他の子があればその状態になる() {
    for kind in [NodeKindName::Sequence, NodeKindName::Fanout] {
        let execution_id = format!("00000000-0000-4000-8000-00000000002{}", kind as u8);
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: "workflow".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "branch".to_string(),
                    node_name: "branch".to_string(),
                    kind,
                    attempt: 1,
                    parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: "unbound-session".to_string(),
                    node_name: "session".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    parent: Some(parent_ref(kind, "branch", 0)),
                    timestamp: 3.0,
                },
            ],
        )
        .unwrap();
        let branch = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("branch"))
            .unwrap();
        assert_eq!(
            branch.status_classification,
            WorkspaceNodeStatusClassification::Unbound,
            "{kind:?}"
        );

        WorkspaceTreeProjector::project(
            &mut tree,
            [WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.clone(),
                node_execution_id: "command".to_string(),
                node_name: "command".to_string(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: Some(parent_ref(kind, "branch", 1)),
                timestamp: 4.0,
            }],
        )
        .unwrap();
        let branch = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("branch"))
            .unwrap();
        assert_eq!(
            branch.status_classification,
            WorkspaceNodeStatusClassification::Active,
            "{kind:?}"
        );
    }
}

#[test]
fn test_親分類集約_bind前sessionの終了状態をsequenceとfanoutへ反映する() {
    let cases = [
        (
            NodeExecutionFailureKind::ValidationFailure,
            WorkspaceNodeStatus::Failed,
            WorkspaceNodeStatusClassification::Failure,
            WorkspaceNodeStatusClassification::Failure,
        ),
        (
            NodeExecutionFailureKind::UserAbort,
            WorkspaceNodeStatus::Aborted,
            WorkspaceNodeStatusClassification::Idle,
            WorkspaceNodeStatusClassification::Active,
        ),
    ];
    for kind in [NodeKindName::Sequence, NodeKindName::Fanout] {
        for (failure_kind, child_status, child_expected, parent_expected) in cases {
            let execution_id = format!(
                "00000000-0000-4000-8000-{:012x}",
                0x300 + kind as u64 * 0x10 + failure_kind as u64
            );
            let mut tree = WorkspaceTree::empty("/repo");
            WorkspaceTreeProjector::project(
                &mut tree,
                [
                    WorkspaceStructureFact::WorkflowStarted {
                        execution_id: execution_id.clone(),
                        workflow_name: "workflow".to_string(),
                        worktree_path: "/repo".to_string(),
                        definition: definition(),
                        timestamp: 1.0,
                    },
                    WorkspaceStructureFact::NodeStarted {
                        execution_id: execution_id.clone(),
                        node_execution_id: "branch".to_string(),
                        node_name: "branch".to_string(),
                        kind,
                        attempt: 1,
                        parent: None,
                        timestamp: 2.0,
                    },
                    WorkspaceStructureFact::NodeStarted {
                        execution_id: execution_id.clone(),
                        node_execution_id: "session".to_string(),
                        node_name: "session".to_string(),
                        kind: NodeKindName::Session,
                        attempt: 1,
                        parent: Some(parent_ref(kind, "branch", 0)),
                        timestamp: 3.0,
                    },
                    WorkspaceStructureFact::NodeFailed {
                        execution_id: execution_id.clone(),
                        node_execution_id: "session".to_string(),
                        reason: "provider launch failed".to_string(),
                        failure_kind,
                        timestamp: 4.0,
                    },
                ],
            )
            .unwrap();

            let session = tree
                .nodes()
                .iter()
                .find(|node| node.node_execution_id.as_deref() == Some("session"))
                .unwrap();
            assert_eq!(session.session_id, None);
            assert_eq!(session.status, child_status, "{kind:?} {failure_kind:?}");
            assert_eq!(
                session.status_classification, child_expected,
                "{kind:?} {failure_kind:?}"
            );
            let branch = tree
                .nodes()
                .iter()
                .find(|node| node.node_execution_id.as_deref() == Some("branch"))
                .unwrap();
            assert_eq!(
                branch.status_classification, parent_expected,
                "{kind:?} {failure_kind:?}"
            );
        }
    }
}

#[test]
fn missing_session_keeps_node_but_returns_no_unusable_session_id() {
    let execution_id = "00000000-0000-4000-8000-000000000012";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: "missing-session".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
        ],
    )
    .unwrap();
    let node = tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("missing-session"))
        .unwrap();
    assert_eq!(node.session_id, None);
    assert!(!node.can_close);
}
