pub(crate) mod tests {
    use super::super::*;

    fn artifact(node_name: &str) -> workflow::Artifact {
        workflow::Artifact {
            node_name: node_name.to_string(),
            contract: Some("result".to_string()),
            value: serde_json::json!({"ok": true}),
            produced_at: 2.0,
        }
    }

    fn node() -> workflow::NodeExecution {
        workflow::NodeExecution {
            process_presence: Default::default(),
            worktree: None,
            id: "node-1".to_string(),
            execution_id: "execution-1".to_string(),
            node_name: "review".to_string(),
            kind: workflow::NodeKindName::Session,
            attempt: 1,
            status: workflow::NodeExecutionStatus::WaitingApproval,
            session_id: Some("session-1".to_string()),
            display_command: None,
            result_summary: None,
            artifact: Some(artifact("review")),
            token_usage: Some(workflow::TokenUsage {
                input_tokens: 3,
                output_tokens: 2,
            }),

            parent: None,
            completion_signals: workflow::NodeCompletionSignalState::StopReceived,
            started_at: 1.5,
            completed_at: None,
        }
    }

    #[test]
    fn maps_complete_public_read_model_without_legacy_wrapper() {
        let node = node();
        let execution = workflow::ExecutionTree {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: workflow::ExecutionStatus::Running,
            current_node: Some("review".to_string()),
            created_from: workflow::ExecutionOrigin::Cli,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: workflow::TokenUsage {
                input_tokens: 3,
                output_tokens: 2,
            },
            node_executions: vec![node.clone()],
            artifacts: vec![artifact("request")],
            fanouts: vec![workflow::Fanout {
                parent: node,
                children: Vec::new(),
                artifact: Some(artifact("review")),
            }],
            approval_target: Some(workflow::ApprovalTarget {
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                session_id: Some("session-1".to_string()),
            }),
        };

        let value = serde_json::to_value(workflow_execution_to_view(execution)).unwrap();
        assert_eq!(value["id"], "execution-1");
        assert_eq!(value["currentNode"], "review");
        assert_eq!(value["nodeExecutions"][0]["artifact"]["nodeName"], "review");
        assert_eq!(value["fanouts"][0]["parent"]["id"], "node-1");
        assert_eq!(value["approvalTarget"]["sessionId"], "session-1");
        assert_eq!(value["nodeExecutions"][0]["submitReceived"], false);
        assert_eq!(value["nodeExecutions"][0]["stopReceived"], true);
        assert_eq!(value["nodeExecutions"][0]["waitingFor"], "submit");
        assert_eq!(value["nodeExecutions"][0]["canApprove"], true);
        assert_eq!(value["nodeExecutions"][0]["canRetry"], false);
        assert_eq!(value["nodeExecutions"][0]["hasArtifact"], true);
        assert!(value.get("interruptionReason").is_none());
        assert!(value.get("resumeFromNode").is_none());
    }

    #[test]
    fn maps_masked_command_display_to_camel_case_wire_field() {
        let mut command = node();
        command.kind = workflow::NodeKindName::Command;
        command.session_id = None;
        command.display_command = Some("printf '[REDACTED]'".to_string());

        let value =
            serde_json::to_value(node_execution_to_view_with_retry(command, false)).unwrap();

        assert_eq!(value["displayCommand"], "printf '[REDACTED]'");
        assert!(value.get("display_command").is_none());
    }
}
