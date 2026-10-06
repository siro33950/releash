use super::*;
use crate::domain::failure::TechnicalFailureNature;
use crate::domain::local_event::WorkflowExecutionMetadataRecord;
use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
use crate::domain::workspace_tree::{
    WorkspaceNodeStatus, WorkspaceNodeStatusClassification, WorkspaceTreeNode,
};

fn node() -> WorkspaceTreeNode {
    WorkspaceTreeNode {
        process_presence: Default::default(),
        can_resume_session: false,
        worktree: None,
        id: "node".to_string(),
        parent_id: None,
        sibling_order: 0,
        kind: WorkspaceNodeKind::WorkflowSession,
        title: "Review".to_string(),
        status: WorkspaceNodeStatus::Waiting,
        status_classification: WorkspaceNodeStatusClassification::Attention,
        delegate_waits_for_child: false,
        background_failure: false,
        activity: Some(crate::domain::workflow::AgentSessionActivity::AwaitingInstruction),
        error_reason: None,
        updated_at_bits: 1.0f64.to_bits(),
        execution_id: None,
        node_execution_id: None,
        node_name: None,
        attempt: Some(1),
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        can_rename: false,
        can_approve: true,
        can_retry: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        command_result: None,
        dynamic_fanout: false,
    }
}

fn child_node(
    id: &str,
    parent_id: &str,
    execution_id: &str,
    kind: WorkspaceNodeKind,
    title: &str,
) -> WorkspaceTreeNode {
    let mut child = node();
    child.id = id.to_string();
    child.parent_id = Some(parent_id.to_string());
    child.kind = kind;
    child.activity = (kind == WorkspaceNodeKind::WorkflowSession)
        .then(crate::domain::workflow::AgentSessionActivity::default);
    child.title = title.to_string();
    child.status = WorkspaceNodeStatus::Running;
    child.execution_id = Some(execution_id.to_string());
    child.node_execution_id = Some(format!("{id}-execution"));
    child.node_name = Some(title.to_string());
    child.can_approve = false;
    child
}

#[test]
fn test_workflow_session_node_detail_session_surfaceを公開する() {
    let mut workflow_session = node();
    workflow_session.session_id = Some("agent-session-1".to_string());

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert_eq!(
        detail["content"]["kind"],
        serde_json::Value::String("session".to_string())
    );
    assert_eq!(
        detail["content"]["sessionId"],
        serde_json::Value::String("agent-session-1".to_string())
    );
}

#[test]
fn workflow_node_detail_exposes_backend_owned_signal_and_capabilities_without_attempt() {
    let mut workflow_session = node();
    workflow_session.node_execution_id = Some("node-execution-1".to_string());
    workflow_session.execution_id = Some("execution-1".to_string());
    workflow_session.node_name = Some("Review".to_string());
    workflow_session.session_id = Some("agent-session-1".to_string());
    workflow_session.completion_signals =
        crate::domain::workflow::NodeCompletionSignalState::StopReceived;
    workflow_session.has_artifact = false;
    workflow_session.can_retry = true;
    workflow_session.can_rename = true;

    let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

    assert!(detail.get("attempt").is_none());
    assert_eq!(detail["submitReceived"], false);
    assert_eq!(detail["stopReceived"], true);
    assert_eq!(detail["waitingFor"], "submit");
    assert_eq!(detail["hasArtifact"], false);
    assert_eq!(detail["capabilities"]["canRetry"], true);
    assert_eq!(detail["capabilities"]["canRename"], true);
}

#[test]
fn test_workspaceノード詳細契約_状態アイコン用分類を返さない() {
    let detail = serde_json::to_value(node_detail(node())).unwrap();
    assert_eq!(detail["status"], "waiting");
    assert!(detail.get("statusClassification").is_none());
}

#[test]
fn execution_summary_rejects_non_finite_timestamp() {
    let record = WorkflowExecutionMetadataRecord {
        execution_id: "execution".to_string(),
        workflow_name: "workflow".to_string(),
        status: ExecutionStatus::Running,
        worktree_path: "/repo".to_string(),
        current_node: None,
        created_from: ExecutionOrigin::DesktopUi,
        started_at_bits: f64::NAN.to_bits(),
        updated_at_bits: 1.0f64.to_bits(),
        completed_at_bits: None,
        error_reason: None,
        total_token_usage: TokenUsage::default(),
    };
    assert!(matches!(
        execution_summary(record),
        Err(WorkflowError::CorruptStoredState(_))
    ));
}

#[test]
fn test_workspace_query_error_corruptをcorrupt_stored_stateへ写像する() {
    // Given
    let correlation_id = "workspace-corrupt-correlation";

    // When
    let error = query_error(crate::domain::local_event::LocalEventQueryError::Corrupt {
        correlation_id: correlation_id.to_string(),
    });

    // Then
    assert_eq!(
        error,
        WorkflowError::CorruptStoredState(format!(
            "store corrupt (correlation_id={correlation_id})"
        ))
    );
}

#[test]
fn unrepresentable_page_offset_falls_back_to_the_first_record() {
    assert_eq!(
        sqlite_page_bounds(Some(WorkflowPageRequest::new(usize::MAX, usize::MAX))),
        (i64::MAX, 0)
    );
    assert_eq!(sqlite_page_bounds(None), (i64::MAX, 0));
}

#[test]
fn test_隔離node詳細_実行中と成果物なし終端でもbranchとpathを公開する() {
    // Given
    let expected = crate::domain::workflow::IsolatedWorktree::for_attempt("/repo", "isolated", 2);
    for status in [WorkspaceNodeStatus::Running, WorkspaceNodeStatus::Aborted] {
        let mut node = child_node(
            "isolated",
            "execution",
            "execution",
            WorkspaceNodeKind::WorkflowSession,
            "work",
        );
        node.status = status;
        node.worktree = Some(expected.clone());
        // When
        let detail = serde_json::to_value(node_detail(node)).unwrap();
        // Then
        assert_eq!(detail["worktree"]["branch"], expected.branch);
        assert_eq!(detail["worktree"]["path"], expected.path);
        assert_eq!(detail["status"], status.as_public_str());
        assert_eq!(detail["hasArtifact"], false);
        assert_eq!(detail["recoveryReason"], serde_json::Value::Null);
    }
}

#[test]
fn test_workspace_query_結果不明と期限切れの分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::LocalEventQueryError;
    use connectrpc::ErrorCode;
    // Given
    for (error, expected) in [
        (
            LocalEventQueryError::CanonicalWriterRequired,
            ErrorCode::Internal,
        ),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
    ] {
        // When / Then
        assert_eq!(query_error(error).connect_code(), expected);
    }
}

#[test]
fn test_workspace_query_store以外の失敗はmainと同じ変種を返す() {
    use crate::adaptor::controller::api::error::ApiError;
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::{
        LocalEventQueryError, SafeOperationFailure, SessionOperationFailureKind,
    };

    for error in [
        LocalEventQueryError::CanonicalWriterRequired,
        LocalEventQueryError::StorageAccessRequired {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                TechnicalFailureNature::Other,
                "access required",
                "id",
            ),
        },
    ] {
        let error = query_error(error);
        assert!(matches!(error, WorkflowError::External(_)));
        assert_eq!(error.connect_code(), connectrpc::ErrorCode::Internal);
        assert_eq!(ApiError::from(error).status.as_u16(), 500);
    }
}

#[test]
fn test_store問い合わせエラー_停止の分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = query_error(crate::domain::local_event::LocalEventQueryError::Technical(
            stopped.into(),
        ));
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
        assert!(matches!(error, WorkflowError::Technical(value) if value == stopped.into()));
    }
}
