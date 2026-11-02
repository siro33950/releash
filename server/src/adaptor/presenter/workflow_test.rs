pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_定義概要_転送型に各項目を保持する() {
        todo!()
    }

    #[test]
    fn test_facet概要_転送型に各項目を保持する() {
        todo!()
    }

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
            shell: None,
            cwd: None,
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
    fn maps_complete_public_read_model() {
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

#[test]
fn test_定義保存結果_成功時はファイル識別名を返す() {
    use super::*;
    use crate::adaptor::presenter::client::save_workflow_source_result_dto::Variant;
    // Given
    let definition = workflow::WorkflowDefinition::restore(
        "file-stem".into(),
        "/workflows".into(),
        workflow::WorkflowSource::File {
            path: "/workflows/file-stem.yml".into(),
        },
        workflow::WorkflowSourceFormat::Yaml,
        Ok(workflow::WorkflowDefinitionContent {
            name: "declared-name".into(),
            ..Default::default()
        }),
    );
    // When
    let result = save_workflow_source_result(Ok(definition)).unwrap();
    // Then
    let Some(Variant::Success(result)) = result.variant else {
        panic!("success")
    };
    assert_eq!(result.ok, Some(true));
    assert_eq!(result.name.as_deref(), Some("file-stem"));
}

#[test]
fn test_定義保存結果_診断と業務失敗を転送する() {
    use super::*;
    use crate::adaptor::presenter::client::save_workflow_source_result_dto::Variant;
    use crate::adaptor::presenter::connect::ConnectFailure;
    use workflow::value_objects::diagnostic::{DiagnosticItem, DiagnosticStage, Severity};
    // Given
    let items = vec![DiagnosticItem::new(
        "WFS001",
        Severity::Error,
        DiagnosticStage::ParseShape,
        None,
        "invalid",
    )];
    // When
    let result = save_workflow_source_result(Err(workflow::WorkflowSourceSaveError::Diagnostics(
        items.clone(),
    )))
    .unwrap();
    // Then
    let Some(Variant::Diagnostics(result)) = result.variant else {
        panic!("diagnostics")
    };
    assert_eq!(result.ok, Some(false));
    assert_eq!(result.error.as_deref(), Some("workflow_diagnostics"));
    assert_eq!(result.diagnostics, Some(items.try_into().unwrap()));
    let error = workflow::WorkflowError::validation("invalid name");
    let result = save_workflow_source_result(Err(workflow::WorkflowSourceSaveError::Workflow(
        error.clone(),
    )))
    .unwrap_err();
    assert_eq!(result.to_string(), error.to_string());
    assert_eq!(
        result.connect_code(),
        connectrpc::ErrorCode::InvalidArgument
    );
}
