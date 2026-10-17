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
        execution_parent: None,
        retry_predecessor_id: None,
        past_attempt_ids: Vec::new(),
        is_retry_history: false,
        completion_signals: Default::default(),
        has_artifact: false,
        session_id: None,
        model: None,
        provider: None,
        can_rename: false,
        can_approve: true,
        can_retry: false,
        can_abort: false,
        can_archive: false,
        display_command: None,
        shell: None,
        cwd: None,
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

    let detail = serde_json::to_value(node_detail(workflow_session, false)).unwrap();

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

    let detail = serde_json::to_value(node_detail(workflow_session, false)).unwrap();

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
    let detail = serde_json::to_value(node_detail(node(), false)).unwrap();
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
        let detail = serde_json::to_value(node_detail(node, false)).unwrap();
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

#[test]
fn test_command詳細_実行時のshellとcwdとabort対象を配る() {
    // Given
    let mut command = node();
    command.kind = WorkspaceNodeKind::WorkflowCommand;
    command.shell = Some("/bin/sh".into());
    command.cwd = Some("/repo/feature".into());
    command.display_command = Some("printf hello".into());
    command.execution_id = Some("execution".into());
    command.command_result = Some(crate::domain::workspace_tree::WorkspaceCommandResult {
        exit_code: 2,
        duration: 1,
        stdout: "out".into(),
        stderr: "err".into(),
    });
    // When
    let mut workflow = node();
    workflow.kind = WorkspaceNodeKind::Workflow;
    workflow.can_abort = true;
    let detail = node_detail_with_workflow(command, Some(&workflow));
    // Then
    assert_eq!(detail.execution_id.as_deref(), Some("execution"));
    assert!(detail.can_abort);
    assert_eq!(
        detail.error_reason.as_deref(),
        Some("Command exited with code 2")
    );
    let WorkspaceNodeContentDto::Command(content) = detail.content else {
        panic!("command expected")
    };
    assert_eq!(content.shell.as_deref(), Some("/bin/sh"));
    assert_eq!(content.cwd.as_deref(), Some("/repo/feature"));
}

#[test]
fn test_node詳細_workflowの節が無くても詳細を返しabort不可とする() {
    // Given
    let mut node = node();
    node.execution_id = Some("missing-execution".into());
    // When
    let detail = node_detail_with_workflow(node, None);
    // Then
    assert_eq!(detail.id, "node");
    assert_eq!(detail.execution_id.as_deref(), Some("missing-execution"));
    assert!(!detail.can_abort);
}

#[test]
fn test_node詳細_完了したworkflowの節のabort不可を配る() {
    // Given
    let mut workflow = node();
    workflow.kind = WorkspaceNodeKind::Workflow;
    workflow.status = WorkspaceNodeStatus::Completed;
    workflow.can_abort = false;
    // When
    let detail = node_detail_with_workflow(node(), Some(&workflow));
    // Then
    assert_eq!(detail.id, "node");
    assert!(!detail.can_abort);
}

#[test]
fn test_node詳細_node実行idと名前とmodelを配る() {
    // Given
    let mut node = node();
    node.node_execution_id = Some("visit-1".into());
    node.node_name = Some("review".into());
    node.model = Some("model".into());
    // When
    let detail = node_detail(node, false);
    // Then
    assert_eq!(detail.classification, "attention");
    assert_eq!(detail.node_execution_id.as_deref(), Some("visit-1"));
    assert_eq!(detail.node_name.as_deref(), Some("review"));
    assert_eq!(detail.model.as_deref(), Some("model"));
}

#[test]
fn test_会話詳細の事実読取_artifact本文を転送せず判定入力と不正な値の拒否を保つ() {
    // Given
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    let (expression, parameters) = session_artifact_projection(vec![
        "done".into(),
        "child.ready".into(),
        "route".into(),
        "items".into(),
    ]);
    let sql = format!("SELECT {expression} FROM (SELECT ?1 AS detail, ?2 AS event_type)");
    for (input, valid) in [
        (
            r#"{"value":{"done":true,"child":{"ready":false,"body":"private"},"body":"private","route":"next","items":["private"]},"contract":"result"}"#,
            true,
        ),
        (r#"{"value":{},"contract":123}"#, false),
        (r#"{"contract":"missing value"}"#, false),
        ("broken", false),
    ] {
        let event_type = "artifact_produced";
        let mut bindings: Vec<&dyn rusqlite::ToSql> = vec![&input, &event_type];
        bindings.extend(parameters.iter().map(|value| value as &dyn rusqlite::ToSql));
        // When
        let result: String = connection
            .query_row(&sql, bindings.as_slice(), |row| row.get(0))
            .unwrap();
        let decoded =
            crate::adaptor::gateway::workflow::fact_codec::decode("artifact_produced", &result);
        // Then
        assert_eq!(decoded.is_ok(), valid);
        if valid {
            assert!(!result.contains("private"));
            assert!(
                matches!(decoded.unwrap(), crate::domain::workflow::NodeFact::ArtifactProduced(fact) if fact.value == serde_json::json!({"done": true, "child.ready": false, "route": "next", "items": 1}) && fact.contract.as_deref() == Some("result"))
            );
        }
    }
}
