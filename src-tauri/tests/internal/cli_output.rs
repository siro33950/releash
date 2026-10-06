use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::cli_common_test_support::append_workflow_event;
use crate::cli_common_test_support::append_workflow_events;
use crate::cli_common_test_support::write_canonical_execution;
use releash_lib::test_support::integration::platform::cmd_output_get;
use releash_lib::test_support::integration::platform::cmd_output_submit;
use releash_lib::test_support::integration::platform::make_execution;
use releash_lib::test_support::integration::platform::test_uuid;
use releash_lib::test_support::integration::platform::CliError;
use releash_lib::test_support::integration::workflow::ExecutionOrigin;
use releash_lib::test_support::integration::workflow::ExecutionStatus;
use releash_lib::test_support::integration::workflow::NodeDefinition;
use releash_lib::test_support::integration::workflow::NodeExecutionFailureKind;
use releash_lib::test_support::integration::workflow::NodeKind;
use releash_lib::test_support::integration::workflow::NodeKindName;
use releash_lib::test_support::integration::workflow::SchemaDef;
use releash_lib::test_support::integration::workflow::SessionSpec;
use releash_lib::test_support::integration::workflow::WorkflowDefinition as WorkflowDefinitionYaml;
use releash_lib::test_support::integration::workflow::WorkflowEvent;
use std::path::Path;
use tempfile::TempDir;

async fn seed_artifact_node(data_dir: &Path, execution_id: &str) {
    let definition = WorkflowDefinitionYaml {
        name: "wf".to_string(),
        description: String::new(),
        builtin: false,
        schemas: BTreeMap::from([(
            "review-verdict".to_string(),
            SchemaDef::Object {
                properties: BTreeMap::from([(
                    "verdict".to_string(),
                    SchemaDef::String { r#enum: None },
                )]),
                required: BTreeSet::from(["verdict".to_string()]),
            },
        )]),
        nodes: vec![NodeDefinition {
            name: "review".to_string(),
            kind: NodeKind::Session(SessionSpec::default()),
            artifact: Some("review-verdict".to_string()),
            ..Default::default()
        }],
        entry: "review".to_string(),
    };
    append_workflow_events(
        data_dir,
        &[
            WorkflowEvent::ExecutionStarted {
                repository_root: None,
                execution_id: execution_id.to_string(),
                workflow_name: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                definition,
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                worktree: None,
                execution_id: execution_id.to_string(),
                node_execution_id: "node-0".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 1.0,
            },
        ],
    )
    .await;
    write_canonical_execution(
        data_dir,
        &make_execution(execution_id, "/repo", ExecutionStatus::Running, 1.0),
    )
    .await;
}

#[tokio::test]
pub async fn test_workflow_output_submit_実行中アプリを要求する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(10);
    seed_artifact_node(temp.path(), &execution_id).await;

    let error = cmd_output_submit(
        temp.path(),
        "550e8400-e29b-41d4-a716-446655440001".to_string(),
        Some("review-verdict"),
        Some(r#"{"verdict":"LGTM"}"#.to_string()),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CliError::Other(message) if message.contains("アプリの起動が必要")
    ));
}

#[tokio::test]
pub async fn test_workflow_output_get_file直接読取で最新artifactを返す() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(12);
    seed_artifact_node(temp.path(), &execution_id).await;
    append_workflow_event(
        temp.path(),
        &WorkflowEvent::ArtifactProduced {
            execution_id: execution_id.clone(),
            node_execution_id: "node-0".to_string(),
            node_name: "review".to_string(),
            contract: Some("review-verdict".to_string()),
            value: serde_json::json!({"verdict": "FIX"}),
            request_id: Some("request-0".to_string()),
            submitted_at: Some(2.0),
            timestamp: 2.0,
        },
    )
    .await;
    append_workflow_events(
        temp.path(),
        &[
            WorkflowEvent::NodeSubmitReceived {
                execution_id: execution_id.clone(),
                node_execution_id: "node-0".to_string(),
                timestamp: 2.25,
            },
            WorkflowEvent::NodeFailed {
                execution_id: execution_id.clone(),
                node_execution_id: "node-0".to_string(),
                node_name: "review".to_string(),
                attempt: 1,
                reason: "retry fixture".to_string(),
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
                timestamp: 2.5,
            },
            WorkflowEvent::NodeRetryRequested {
                execution_id: execution_id.clone(),
                node_execution_id: "node-0".to_string(),
                timestamp: 2.75,
            },
            WorkflowEvent::NodeStarted {
                worktree: None,
                execution_id: execution_id.clone(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 2,
                parent: None,
                timestamp: 2.75,
            },
            WorkflowEvent::ArtifactProduced {
                execution_id: execution_id.clone(),
                node_execution_id: "node-1".to_string(),
                node_name: "review".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({"verdict": "LGTM"}),
                request_id: Some("request-1".to_string()),
                submitted_at: Some(3.0),
                timestamp: 3.0,
            },
        ],
    )
    .await;

    let output = cmd_output_get(temp.path(), &execution_id, "review", true)
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["status"], "submitted");
    assert_eq!(output["artifact"]["verdict"], "LGTM");
    assert_eq!(output["request_id"], "request-1");
    assert!(output.get("structured_output").is_none());
}

#[tokio::test]
pub async fn test_隔離worktree_output_getのfile直接読取で完了したattemptのbranchとpathを返す() {
    use crate::adaptor_controller_api_mod::test_support::isolated_worktree_json;
    use crate::adaptor_controller_api_mod::test_support::seed_isolated_query_execution;
    use releash_lib::test_support::integration::workflow::NodeExecutionStatus;

    // Given
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(33);
    seed_isolated_query_execution(temp.path(), &execution_id, NodeExecutionStatus::Succeeded).await;

    // When
    let output = cmd_output_get(temp.path(), &execution_id, "review", true)
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();

    // Then
    assert_eq!(output["status"], "submitted");
    assert_eq!(output["contract"], "review-result");
    assert_eq!(output["artifact"]["status"], "approved");
    assert_eq!(output["artifact"]["worktree"], isolated_worktree_json());
    assert_eq!(output["request_id"], "isolated-request-2");
}

#[tokio::test]
pub async fn test_workflow_output_get_既知nodeで未提出を報告する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(13);
    seed_artifact_node(temp.path(), &execution_id).await;

    assert_eq!(
        cmd_output_get(temp.path(), &execution_id, "review", false)
            .await
            .unwrap(),
        "not_submitted: node=review\n"
    );
}

#[tokio::test]
pub async fn test_workflow_output_get_未知nodeをfile直接読取で拒否する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(14);
    seed_artifact_node(temp.path(), &execution_id).await;
    assert!(matches!(
        cmd_output_get(temp.path(), &execution_id, "missing", true).await,
        Err(CliError::InvalidInput(_))
    ));
}
