use super::*;
use crate::domain::workflow::ExecutionTreeLaunch;
use crate::domain::workflow::{ExecutionStatus, NodeKindName};
use crate::domain::workspace_tree::{
    WorkspaceExecution, WorkspaceStructureFact, WorkspaceTreeProjector,
};
use crate::domain::workspace_tree::{WorkspaceNodeStatusClassification, WorkspaceTree};

#[test]
fn test_カード集計_実行木単位で過去の試行と構造nodeとarchiveを除く() {
    // Given
    let id = "00000000-0000-4000-8000-000000001204";
    let mut tree = WorkspaceTree::empty("/repo");
    let mut facts = vec![WorkspaceStructureFact::WorkflowStarted {
        execution_id: id.into(),
        workflow_name: "review".into(),
        worktree_path: "/repo".into(),
        dynamic_fanout_names: Default::default(),
        timestamp: 1.0,
    }];
    for (name, kind) in [
        ("s1", NodeKindName::Session),
        ("s2", NodeKindName::Session),
        ("cmd", NodeKindName::Command),
        ("sequence", NodeKindName::Sequence),
    ] {
        facts.push(WorkspaceStructureFact::NodeStarted {
            execution_id: id.into(),
            node_execution_id: name.into(),
            node_name: name.into(),
            kind,
            attempt: 1,
            parent: None,
            timestamp: 2.0,
        });
    }
    WorkspaceTreeProjector::project(&mut tree, facts).unwrap();
    tree.record_executions(vec![WorkspaceExecution {
        execution_id: id.into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        worktree_path: "/repo".into(),
        workflow_name: "review".into(),
        status: ExecutionStatus::Running,
        updated_at: 2.0,
        archive: None,
        session: None,
    }]);
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::NodeCompleted {
                execution_id: id.into(),
                node_execution_id: "s1".into(),
                timestamp: 2.5,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: id.into(),
                node_execution_id: "s1-retry".into(),
                node_name: "s1".into(),
                kind: NodeKindName::Session,
                attempt: 2,
                parent: None,
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeRetryLinked {
                execution_id: id.into(),
                node_execution_id: "s1-retry".into(),
                predecessor_node_execution_id: "s1".into(),
            },
        ],
    )
    .unwrap();
    tree.observe_background_failure("s2", "needs attention");
    // When
    let summaries = tree.card_executions();
    // Then
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].1.node_count(), 3);
    assert_eq!(summaries[0].1.root.public_title(), "review");
    assert_eq!(
        summaries[0].1.session_states().collect::<Vec<_>>(),
        [
            WorkspaceNodeStatusClassification::Attention,
            WorkspaceNodeStatusClassification::Active
        ]
    );
    assert_eq!(
        summaries[0].1.root.public_status(),
        WorkspaceNodeStatusClassification::Attention
    );
    assert_eq!(
        tree.card_status(),
        Some(WorkspaceNodeStatusClassification::Attention)
    );
    assert_eq!(WorkspaceTree::empty("/repo").card_status(), None);
}

#[test]
fn test_カード集計_archive済みの実行木は行にも状態にも含めない() {
    // Given
    let id = "00000000-0000-4000-8000-000000001204";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::WorkflowStarted {
            execution_id: id.into(),
            workflow_name: "review".into(),
            worktree_path: "/repo".into(),
            dynamic_fanout_names: Default::default(),
            timestamp: 1.0,
        }],
    )
    .unwrap();
    tree.record_executions(vec![WorkspaceExecution {
        execution_id: id.into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        worktree_path: "/repo".into(),
        workflow_name: "review".into(),
        status: ExecutionStatus::Running,
        updated_at: 2.0,
        archive: Some(crate::domain::workflow::ExecutionTreeArchiveRecord {
            execution_id: id.into(),
            archived_at: 3.0,
            archive_reason: "user".into(),
        }),
        session: None,
    }]);
    // When / Then
    assert!(tree.card_executions().is_empty());
    assert_eq!(tree.card_status(), None);
}

#[test]
fn test_カード集計_session行を先に並べ人の番を集約する() {
    // Given
    let workflow = "00000000-0000-4000-8000-000000001205";
    let session = "00000000-0000-4000-8000-000000001206";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: workflow.into(),
                workflow_name: "review".into(),
                worktree_path: "/repo".into(),
                dynamic_fanout_names: Default::default(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: session.into(),
                workflow_name: "session".into(),
                worktree_path: "/repo".into(),
                dynamic_fanout_names: Default::default(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: session.into(),
                node_execution_id: session.into(),
                node_name: "session".into(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeSessionDisplayNameProjected {
                execution_id: session.into(),
                node_execution_id: session.into(),
                manual_name: Some("作業session".into()),
                provider_session_title: None,
            },
        ],
    )
    .unwrap();
    tree.record_executions(vec![
        WorkspaceExecution {
            execution_id: workflow.into(),
            launched_as: ExecutionTreeLaunch::Workflow,
            worktree_path: "/repo".into(),
            workflow_name: "review".into(),
            status: ExecutionStatus::Running,
            updated_at: 2.0,
            archive: None,
            session: None,
        },
        WorkspaceExecution {
            execution_id: session.into(),
            launched_as: ExecutionTreeLaunch::Session,
            worktree_path: "/repo".into(),
            workflow_name: "session".into(),
            status: ExecutionStatus::Running,
            updated_at: 2.0,
            archive: None,
            session: Some(crate::domain::agent_session::aggregates::AgentSession::create(
                session, crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"), "/repo",
                crate::domain::provider_lifecycle::ProviderKind::Codex,
                crate::domain::agent_session::aggregates::AgentSessionTreeLocation::session_tree_root(session).unwrap(),
            ).unwrap()),
        },
    ]);
    tree.observe_background_failure(workflow, "failed");
    // When
    let rows = tree.card_executions();
    // Then
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0.execution_id, session);
    assert!(!ExecutionNodes::is_workflow(rows[0].0.launched_as));
    assert_eq!(rows[0].1.root.public_title(), "作業session");
    assert_eq!(
        rows[0].0.session.as_ref().map(|session| session.provider()),
        Some(crate::domain::provider_lifecycle::ProviderKind::Codex)
    );
    assert_eq!(
        rows[0].1.root.public_status(),
        WorkspaceNodeStatusClassification::Active
    );
    assert_eq!(
        rows[1].1.root.public_status(),
        WorkspaceNodeStatusClassification::Attention
    );
    assert!(ExecutionNodes::is_workflow(rows[1].0.launched_as));
    assert_eq!(
        tree.card_status(),
        Some(WorkspaceNodeStatusClassification::Attention)
    );
}

#[test]
fn test_カード集計_入力待ちの単独sessionは行も集約もidleになる() {
    // Given
    let id = "00000000-0000-4000-8000-000000001204";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::WorkflowStarted {
                execution_id: id.into(),
                workflow_name: "session".into(),
                worktree_path: "/repo".into(),
                dynamic_fanout_names: Default::default(),
                timestamp: 1.0,
            },
            WorkspaceStructureFact::NodeStarted {
                execution_id: id.into(),
                node_execution_id: id.into(),
                node_name: "session".into(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: id.into(),
                node_execution_id: id.into(),
                timestamp: 3.0,
            },
            WorkspaceStructureFact::NodeAgentBound {
                execution_id: id.into(),
                node_execution_id: id.into(),
                session_id: "agent".into(),
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeActivityProjected {
                execution_id: id.into(),
                node_execution_id: id.into(),
                activity: crate::domain::workflow::AgentSessionActivity::AwaitingInstruction,
            },
        ],
    )
    .unwrap();
    tree.record_executions(vec![WorkspaceExecution {
        execution_id: id.into(),
        launched_as: ExecutionTreeLaunch::Session,
        worktree_path: "/repo".into(),
        workflow_name: "session".into(),
        status: ExecutionStatus::Running,
        updated_at: 3.0,
        archive: None,
        session: None,
    }]);
    // When
    let rows = tree.card_executions();
    // Then
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].1.root.owner().status_classification,
        WorkspaceNodeStatusClassification::Active
    );
    assert_eq!(
        rows[0].1.root.public_status(),
        WorkspaceNodeStatusClassification::Idle
    );
    assert_eq!(
        tree.card_status(),
        Some(WorkspaceNodeStatusClassification::Idle)
    );
}

#[test]
fn test_カード集計_子のないworkflowもownerの状態で行と集約に含める() {
    // Given
    let id = "00000000-0000-4000-8000-000000001204";
    let mut tree = WorkspaceTree::empty("/repo");
    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceStructureFact::WorkflowStarted {
            execution_id: id.into(),
            workflow_name: "review".into(),
            worktree_path: "/repo".into(),
            dynamic_fanout_names: Default::default(),
            timestamp: 1.0,
        }],
    )
    .unwrap();
    tree.record_executions(vec![WorkspaceExecution {
        execution_id: id.into(),
        launched_as: ExecutionTreeLaunch::Workflow,
        worktree_path: "/repo".into(),
        workflow_name: "review".into(),
        status: ExecutionStatus::Running,
        updated_at: 1.0,
        archive: None,
        session: None,
    }]);
    // When
    let rows = tree.card_executions();
    // Then
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.root.public_title(), "review");
    assert_eq!(
        rows[0].1.root.public_status(),
        rows[0].1.root.owner().status_classification
    );
    assert_eq!(
        tree.card_status(),
        Some(WorkspaceNodeStatusClassification::Active)
    );
    drop(rows);
    WorkspaceTreeProjector::project(
        &mut tree,
        [
            WorkspaceStructureFact::NodeStarted {
                execution_id: id.into(),
                node_execution_id: "child".into(),
                node_name: "child".into(),
                kind: NodeKindName::Command,
                attempt: 1,
                parent: None,
                timestamp: 2.0,
            },
            WorkspaceStructureFact::NodeCompleted {
                execution_id: id.into(),
                node_execution_id: "child".into(),
                timestamp: 3.0,
            },
        ],
    )
    .unwrap();
    let rows = tree.card_executions();
    assert_eq!(
        rows[0].1.root.node().status_classification,
        WorkspaceNodeStatusClassification::Idle
    );
    assert_eq!(
        rows[0].1.root.public_status(),
        WorkspaceNodeStatusClassification::Active
    );
    assert_eq!(
        tree.card_status(),
        Some(WorkspaceNodeStatusClassification::Active)
    );
}
