use super::tests::{execution, node, EXECUTION_ID};
use super::*;
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
use crate::domain::workflow::{NodeCompletionSignalState, NodeKindName, WorkflowDefinition};
use crate::domain::workspace_tree::{WorkspaceNodeStatus, WorkspaceNodeStatusClassification};

#[test]
fn test_未紐づけsession_プロセス不在判定でも青になる() {
    use crate::domain::workflow::NodeProcessPresence;
    // Given
    let mut runtime = node("leaf", EXECUTION_ID, RuntimeNodeExecutionStatus::Running);
    runtime.kind = NodeKindName::Session;
    runtime.display_command = None;
    // When
    let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
        process_presences: &[("leaf".into(), NodeProcessPresence::ConfirmedAbsent)]
            .into_iter()
            .collect(),
        execution_id: EXECUTION_ID,
        workflow_name: "workflow",
        workspace_identity: "/repo",
        workflow_definition: Some(&WorkflowDefinition::default()),
        node_executions: &[runtime],
        retry_predecessors: &Default::default(),
        delegate_waiting_node_ids: &Default::default(),
        execution_active: true,
        started_at: 1.0,
        updated_at: 2.0,
        execution: &execution(),
        session_activities: &Default::default(),
        session_display_names: &Default::default(),
    })
    .unwrap();
    let session = nodes
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("leaf"))
        .unwrap();
    // Then
    assert_eq!(session.session_id, None);
    assert_eq!(
        session.process_presence,
        NodeProcessPresence::ConfirmedAbsent
    );
    assert_eq!(
        session.status_classification,
        WorkspaceNodeStatusClassification::Active
    );
}

#[test]
fn test_完了済みworkflow_session_プロセス消失後も緑になる() {
    use crate::domain::workflow::NodeProcessPresence as P;
    // Given
    let mut runtime = node(
        "workflow-session",
        EXECUTION_ID,
        RuntimeNodeExecutionStatus::Succeeded,
    );
    runtime.kind = NodeKindName::Session;
    runtime.session_id = Some("session".into());
    runtime.display_command = None;
    let activities = [(
        "workflow-session".into(),
        crate::domain::workflow::AgentSessionActivity::Working,
    )]
    .into_iter()
    .collect();
    for (presence, expected) in [
        (P::Live, WorkspaceNodeStatusClassification::Active),
        (P::ConfirmedAbsent, WorkspaceNodeStatusClassification::Idle),
    ] {
        // When
        let nodes = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            process_presences: &[("workflow-session".into(), presence)]
                .into_iter()
                .collect(),
            execution_id: EXECUTION_ID,
            workflow_name: "session",
            workspace_identity: "/repo",
            workflow_definition: Some(&WorkflowDefinition::default()),
            node_executions: std::slice::from_ref(&runtime),
            retry_predecessors: &Default::default(),
            delegate_waiting_node_ids: &Default::default(),
            execution_active: true,
            started_at: 1.0,
            updated_at: 2.0,
            execution: &execution(),
            session_activities: &activities,
            session_display_names: &Default::default(),
        })
        .unwrap();
        let session = nodes
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("workflow-session"))
            .unwrap();
        // Then
        assert!(!session.is_standalone_session_root());
        assert_eq!(session.status, WorkspaceNodeStatus::Completed);
        assert_eq!(
            session.activity,
            Some(crate::domain::workflow::AgentSessionActivity::Working)
        );
        assert_eq!(session.status_classification, expected, "{presence:?}");
    }
}

#[test]
fn test_delegate親_子のactivityを集約する() {
    // Given
    let mut parent = node("parent", EXECUTION_ID, RuntimeNodeExecutionStatus::Running);
    parent.kind = NodeKindName::Session;
    parent.session_id = Some("parent-session".into());
    parent.display_command = None;
    parent.completion_signals = NodeCompletionSignalState::Ready;
    let mut child = node("child", EXECUTION_ID, RuntimeNodeExecutionStatus::Running);
    child.kind = NodeKindName::Session;
    child.session_id = Some("child-session".into());
    child.display_command = None;
    child.parent = Some(crate::domain::workflow::ExecutionParentRef::delegate_child(
        "parent",
    ));
    let nodes = [parent, child];
    for (activity, expected) in [
        (
            crate::domain::workflow::AgentSessionActivity::Working,
            WorkspaceNodeStatusClassification::Active,
        ),
        (
            crate::domain::workflow::AgentSessionActivity::AwaitingAnswer,
            WorkspaceNodeStatusClassification::Attention,
        ),
    ] {
        // When
        let session_activities = [("child".into(), activity)].into_iter().collect();
        let projected = runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            process_presences: &Default::default(),
            execution_id: EXECUTION_ID,
            workflow_name: "workflow",
            workspace_identity: "/repo",
            workflow_definition: Some(&WorkflowDefinition::default()),
            node_executions: &nodes,
            retry_predecessors: &Default::default(),
            delegate_waiting_node_ids: &["parent".into()].into_iter().collect(),
            execution_active: true,
            started_at: 1.0,
            updated_at: 2.0,
            execution: &execution(),
            session_activities: &session_activities,
            session_display_names: &Default::default(),
        })
        .unwrap();
        let parent = projected
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("parent"))
            .unwrap();
        // Then
        assert_eq!(parent.status_classification, expected);
    }
}
