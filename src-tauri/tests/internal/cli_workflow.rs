use crate::cli_common_test_support::append_workflow_events;
use crate::cli_common_test_support::initialize_canonical_store;
use crate::cli_common_test_support::write_canonical_execution;
use releash_lib::test_support::integration::platform::execution_started_event;
use releash_lib::test_support::integration::platform::make_execution;
use releash_lib::test_support::integration::platform::root_node_started_event;
use releash_lib::test_support::integration::platform::test_uuid;
use releash_lib::test_support::integration::platform::CliError;
use releash_lib::test_support::integration::workflow::cmd_status;
use releash_lib::test_support::integration::workflow::ExecutionStatus;
use std::path::Path;
use tempfile::TempDir;

async fn seed_execution(data_dir: &Path, execution_id: &str) {
    write_canonical_execution(
        data_dir,
        &make_execution(execution_id, "/repo", ExecutionStatus::Running, 100.0),
    )
    .await;
    append_workflow_events(
        data_dir,
        &[
            execution_started_event(execution_id, "wf", "/repo"),
            root_node_started_event(execution_id, "ne-main-1", "main", 100.0),
        ],
    )
    .await;
}

#[tokio::test]
pub async fn test_workflow_status_アプリ停止中はfile直接読取へfallbackする() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(1);
    seed_execution(temp.path(), &execution_id).await;

    let status = cmd_status(temp.path(), &execution_id, true).await.unwrap();
    let status: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status["id"], execution_id);
    assert_eq!(status["status"], "running");
    assert_eq!(status["artifacts"][0]["nodeName"], "request");
}

#[tokio::test]
pub async fn test_隔離worktree_statusのfile直接読取で実行中とabortと完了後のbranchとpathを返す() {
    use crate::adaptor_controller_api_mod::test_support::assert_isolated_execution_json;
    use crate::adaptor_controller_api_mod::test_support::seed_isolated_query_execution;
    use releash_lib::test_support::integration::workflow::NodeExecutionStatus;

    for status in [
        NodeExecutionStatus::Running,
        NodeExecutionStatus::Aborted,
        NodeExecutionStatus::Succeeded,
    ] {
        // Given
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(33);
        seed_isolated_query_execution(temp.path(), &execution_id, status).await;

        // When
        let output = cmd_status(temp.path(), &execution_id, true).await.unwrap();
        let output: serde_json::Value = serde_json::from_str(&output).unwrap();

        // Then
        assert_eq!(output["id"], execution_id);
        assert_isolated_execution_json(&output, status);
    }
}

#[tokio::test]
pub async fn test_workflow_status_file直接読取とtauriが同じprojectionを返す() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(2);
    seed_execution(temp.path(), &execution_id).await;

    let cli = releash_lib::test_support::integration::platform::execution_status(
        temp.path(),
        &execution_id,
    )
    .await
    .unwrap();
    let tauri =
        releash_lib::test_support::integration::platform::build_workflow_usecase(temp.path(), None)
            .read_usecase()
            .get_execution_state(&execution_id)
            .await
            .unwrap()
            .map(releash_lib::test_support::integration::workflow::workflow_execution_to_view)
            .unwrap();
    assert_eq!(cli, tauri);
}

#[tokio::test]
pub async fn test_workflow_status_存在しないexecutionを状態作成せずに報告する() {
    let temp = TempDir::new().unwrap();
    initialize_canonical_store(temp.path());
    let execution_id = test_uuid(3);
    let error = cmd_status(temp.path(), &execution_id, false)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CliError::NotFound(format!("Workflow execution not found: {execution_id}"))
    );
}

#[tokio::test]
pub async fn test_workflow_status_jsonは三状態の値を保ち中断理由と再開位置を含まない() {
    // Given / When / Then
    for state in [
        ExecutionStatus::Running,
        ExecutionStatus::Completed,
        ExecutionStatus::Aborted,
    ] {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(4);
        let initial = if state == ExecutionStatus::Aborted {
            ExecutionStatus::Running
        } else {
            state
        };
        write_canonical_execution(
            temp.path(),
            &make_execution(&execution_id, "/repo", initial, 100.0),
        )
        .await;
        if state == ExecutionStatus::Aborted {
            append_workflow_events(
                temp.path(),
                &[releash_lib::test_support::integration::workflow::WorkflowEvent::ExecutionAborted {
                    execution_id: execution_id.clone(),
                    aborted_node: None,
                    timestamp: 101.0,
                }],
            )
            .await;
        }
        let output = cmd_status(temp.path(), &execution_id, true).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["id"], execution_id);
        assert_eq!(value["status"], state.as_str());
        assert!(value.get("interruptionReason").is_none());
        assert!(value.get("resumeFromNode").is_none());
    }
}
