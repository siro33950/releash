use std::collections::{BTreeMap, BTreeSet};

use super::super::common::test_support::{
    append_workflow_event, make_execution, test_uuid, write_execution_file,
};
use super::super::Cli;
use super::*;
use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::schema::{
    NodeDefinition, NodeKind, SchemaDef, SessionSpec, WorkflowDefinitionYaml,
};
use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};
use clap::Parser;
use tempfile::TempDir;

fn seed_artifact_node(data_dir: &Path, execution_id: &str) {
    write_execution_file(
        data_dir,
        &make_execution(execution_id, "/repo", ExecutionStatus::Running, 1.0),
    );
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
    };
    append_workflow_event(
        data_dir,
        &WorkflowEvent::ExecutionStarted {
            execution_id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::Cli,
            request: String::new(),
            definition,
            timestamp: 1.0,
        },
    );
}

#[test]
fn test_workflow_output_cli_現在のcommandをparseする() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";
    for argv in [
        vec![
            "releash",
            "workflow",
            "output",
            "submit",
            "--node-execution",
            "550e8400-e29b-41d4-a716-446655440001",
            "--type",
            "review-verdict",
            "--json",
            r#"{"verdict":"LGTM"}"#,
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
        ],
    ] {
        assert!(Cli::try_parse_from(argv).is_ok());
    }
}

#[test]
fn test_workflow_output_get_未知optionを拒否する() {
    let execution_id = "550e8400-e29b-41d4-a716-446655440000";

    for argv in [
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-flag",
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-option",
            "ignored",
        ],
        vec![
            "releash",
            "workflow",
            "output",
            "get",
            execution_id,
            "--node",
            "review",
            "--future-option=ignored",
        ],
    ] {
        let error = Cli::try_parse_from(argv.clone()).unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "unknown option must be rejected: {argv:?}"
        );
    }
}

#[test]
fn test_workflow_output_submit_requires_attempt_identity_and_accepts_optional_artifact() {
    let node_execution_id = "550e8400-e29b-41d4-a716-446655440001";

    assert!(Cli::try_parse_from([
        "releash",
        "workflow",
        "output",
        "submit",
        "--node-execution",
        node_execution_id,
    ])
    .is_ok());
    assert!(Cli::try_parse_from([
        "releash",
        "workflow",
        "output",
        "submit",
        "--node-execution",
        node_execution_id,
        "--type",
        "review-verdict",
    ])
    .is_err());
    assert!(Cli::try_parse_from(["releash", "workflow", "output", "submit"]).is_err());
}

#[test]
fn test_workflow_output_submit_rejects_blank_node_execution_id() {
    let temp = TempDir::new().unwrap();

    assert_eq!(
        cmd_output_submit(temp.path(), "   ".to_string(), None, None, None).unwrap_err(),
        CliError::InvalidInput("--node-execution must not be empty".to_string())
    );
}

#[test]
fn test_workflow_output_submit_実行中アプリを要求する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(10);
    seed_artifact_node(temp.path(), &execution_id);

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

#[test]
fn test_workflow_output_get_file直接読取で最新artifactを返す() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(12);
    seed_artifact_node(temp.path(), &execution_id);
    for (index, verdict) in ["FIX", "LGTM"].into_iter().enumerate() {
        append_workflow_event(
            temp.path(),
            &WorkflowEvent::ArtifactProduced {
                execution_id: execution_id.clone(),
                node_execution_id: format!("node-{index}"),
                node_name: "review".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({"verdict": verdict}),
                request_id: Some(format!("request-{index}")),
                submitted_at: Some(2.0 + index as f64),
                timestamp: 2.0 + index as f64,
            },
        );
    }

    let output = cmd_output_get(temp.path(), &execution_id, "review", true).unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["status"], "submitted");
    assert_eq!(output["artifact"]["verdict"], "LGTM");
    assert_eq!(output["request_id"], "request-1");
    assert!(output.get("structured_output").is_none());
}

#[test]
fn test_workflow_output_get_既知nodeで未提出を報告する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(13);
    seed_artifact_node(temp.path(), &execution_id);

    assert_eq!(
        cmd_output_get(temp.path(), &execution_id, "review", false).unwrap(),
        "not_submitted: node=review\n"
    );
}

#[test]
fn test_workflow_output_get_未知nodeをfile直接読取で拒否する() {
    let temp = TempDir::new().unwrap();
    let execution_id = test_uuid(14);
    seed_artifact_node(temp.path(), &execution_id);
    assert!(matches!(
        cmd_output_get(temp.path(), &execution_id, "missing", true),
        Err(CliError::InvalidInput(_))
    ));
}
