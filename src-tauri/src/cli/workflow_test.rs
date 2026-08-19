use super::super::common::test_support::{
    append_workflow_events, execution_started_event, initialize_canonical_store, make_execution,
    root_node_started_event, test_uuid, write_execution_file,
};
use super::super::Cli;
use super::*;
use crate::domain::workflow::ExecutionStatus;
use clap::Parser;
use tempfile::TempDir;

fn seed_execution(data_dir: &Path, execution_id: &str) {
    write_execution_file(
        data_dir,
        &make_execution(execution_id, "/repo", ExecutionStatus::Running, 100.0),
    );
    append_workflow_events(
        data_dir,
        &[
            execution_started_event(execution_id, "wf", "/repo"),
            root_node_started_event(execution_id, "ne-main-1", "main", 100.0),
        ],
    );
}

#[test]
fn test_workflow_status_cli_正規語彙でparseできる() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    assert!(Cli::try_parse_from(["releash", "workflow", "status", execution_id, "--json"]).is_ok());
}

#[test]
fn test_workflow_status_アプリ停止中はfile直接読取へfallbackする() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(1);
    seed_execution(temp.path(), &execution_id);

    let status = cmd_status(temp.path(), &execution_id, true).unwrap();
    let status: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status["id"], execution_id);
    assert_eq!(status["status"], "running");
    assert_eq!(status["artifacts"][0]["nodeName"], "request");
}

#[test]
fn test_workflow_status_file直接読取とtauriが同じprojectionを返す() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(2);
    seed_execution(temp.path(), &execution_id);

    let cli = file_direct::execution_status(temp.path(), &execution_id).unwrap();
    let tauri = crate::adaptor::controller::wiring::build_workflow_usecase(temp.path())
        .get_execution_state(&execution_id)
        .unwrap()
        .map(crate::adaptor::presenter::workflow::workflow_execution_to_view)
        .unwrap();
    assert_eq!(cli, tauri);
}

#[test]
fn test_workflow_status_存在しないexecutionを状態作成せずに報告する() {
    let temp = TempDir::new().unwrap();
    initialize_canonical_store(temp.path());
    let execution_id = test_uuid(3);
    let error = cmd_status(temp.path(), &execution_id, false).unwrap_err();
    assert_eq!(
        error,
        CliError::NotFound(format!("Workflow execution not found: {execution_id}"))
    );
}
