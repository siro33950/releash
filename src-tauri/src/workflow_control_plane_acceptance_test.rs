use super::*;

#[test]
fn test_実行状態受信_unresolvedを含むnode状態を変換できる() {
    // Given
    for (status, expected) in [
        ("unresolved", AcceptanceNodeExecutionStatus::Unresolved),
        ("running", AcceptanceNodeExecutionStatus::Running),
        ("paused", AcceptanceNodeExecutionStatus::Paused),
        (
            "waiting_approval",
            AcceptanceNodeExecutionStatus::WaitingApproval,
        ),
        ("succeeded", AcceptanceNodeExecutionStatus::Succeeded),
        ("failed", AcceptanceNodeExecutionStatus::Failed),
        ("aborted", AcceptanceNodeExecutionStatus::Aborted),
    ] {
        let body = serde_json::json!({
            "id": "tree", "status": "running", "nodeExecutions": [{
                "id": "node", "nodeName": "worker", "kind": "session", "attempt": 1,
                "status": status, "sessionId": "session", "submitReceived": false,
                "stopReceived": false, "canApprove": false, "canRetry": false,
                "hasArtifact": false, "artifact": null, "failure": null,
                "recoveryReason": "unavailable definition"
            }]
        });

        // When
        let response: ExecutionResponse = serde_json::from_value(body).unwrap();
        let execution = AcceptanceWorkflowExecution::from(response);

        // Then
        assert_eq!(execution.node_executions[0].status, expected);
        assert_eq!(
            execution.node_executions[0].agent_session_id.as_deref(),
            Some("session")
        );
    }
}

#[test]
fn test_workflow状態受信_三状態だけを受理する() {
    // Given / When / Then
    for (status, expected) in [
        ("running", AcceptanceWorkflowExecutionStatus::Running),
        ("completed", AcceptanceWorkflowExecutionStatus::Completed),
        ("aborted", AcceptanceWorkflowExecutionStatus::Aborted),
    ] {
        let response: ExecutionResponse = serde_json::from_value(serde_json::json!({
            "id": "tree", "status": status, "nodeExecutions": []
        }))
        .unwrap();
        assert_eq!(AcceptanceWorkflowExecution::from(response).status, expected);
    }
    for status in ["waiting_approval", "interrupted"] {
        assert!(
            serde_json::from_value::<ExecutionResponse>(serde_json::json!({
                "id": "tree", "status": status, "nodeExecutions": []
            }))
            .is_err()
        );
    }
}
