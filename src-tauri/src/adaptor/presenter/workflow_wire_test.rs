pub(crate) mod tests {
    use super::super::*;

    fn execution() -> WorkflowExecutionView {
        WorkflowExecutionView {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatusView::Running,
            current_node: Some("review".to_string()),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOriginView::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsageView::default(),
            node_executions: Vec::new(),
            artifacts: vec![ArtifactView {
                node_name: "request".to_string(),
                contract: None,
                value: serde_json::Value::String("review".to_string()),
                produced_at: 1.0,
            }],
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    #[test]
    fn workflow_execution_uses_canonical_camel_case_boundary() {
        let value = serde_json::to_value(execution()).unwrap();
        assert_eq!(value["id"], "execution-1");
        assert_eq!(value["status"], "running");
        assert_eq!(value["createdFrom"], "cli");
        assert!(value.get("interruptionReason").is_none());
        assert!(value.get("resumeFromNode").is_none());
        assert_eq!(value["artifacts"][0]["nodeName"], "request");
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "approvalTarget",
                "artifacts",
                "completedAt",
                "createdFrom",
                "currentNode",
                "errorReason",
                "fanouts",
                "id",
                "nodeExecutions",
                "startedAt",
                "status",
                "totalTokenUsage",
                "updatedAt",
                "workflowName",
                "worktreePath",
            ]
        );
    }
}
