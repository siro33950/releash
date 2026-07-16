use std::path::{Path, PathBuf};

use clap::Subcommand;

use super::api_client;
use super::common::{validate_execution_id, validate_node, CliError};
use super::file_direct;
use crate::adaptor::controller::api::protocol::{SubmitArtifactRequest, ValidateArtifactRequest};
use crate::usecase::workflow::{WorkflowGetOutputResult, WorkflowValidateOutputResult};

#[derive(Subcommand, Debug)]
pub(super) enum OutputSubcommand {
    /// node の Artifact schema に従う値を提出する。
    Submit {
        execution_id: String,
        #[arg(long, value_name = "NODE_NAME")]
        node: String,
        #[arg(long = "node-execution", value_name = "NODE_EXECUTION_ID")]
        node_execution: Option<String>,
        #[arg(long = "type", value_name = "CONTRACT")]
        contract: String,
        #[arg(long, conflicts_with = "file", value_name = "JSON")]
        json: Option<String>,
        #[arg(long, conflicts_with = "json", value_name = "PATH")]
        file: Option<PathBuf>,
    },
    /// Artifact schema 適合性を副作用なしで確認する。
    Validate {
        execution_id: String,
        #[arg(long, value_name = "NODE_NAME")]
        node: String,
        #[arg(long = "type", value_name = "CONTRACT")]
        contract: String,
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },
    /// 提出済み Artifact を取得する。
    Get {
        execution_id: String,
        #[arg(long, value_name = "NODE_NAME")]
        node: String,
        #[arg(long)]
        json: bool,
    },
}

pub(super) fn cmd_output_submit(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    node_execution: Option<String>,
    contract: &str,
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node(node)?;
    validate_contract_argument(contract)?;
    let value = parse_json_input(read_submit_input_json(json_arg, file_arg)?)?;
    let request = SubmitArtifactRequest {
        node: node.to_string(),
        node_execution_id: api_client::resolve_node_execution_id(node_execution),
        contract: contract.to_string(),
        value,
    };
    api_client::mutation(data_dir, |client| {
        client.submit_output(execution_id, &request)
    })?;
    Ok(format!(
        "submitted: execution_id={execution_id} node={node} type={contract}\n"
    ))
}

pub(super) fn cmd_output_validate(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    contract: &str,
    file: &Path,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node(node)?;
    validate_contract_argument(contract)?;
    let value = parse_json_input(read_file(file)?)?;
    let request = ValidateArtifactRequest {
        node: node.to_string(),
        contract: contract.to_string(),
        value: value.clone(),
    };
    let response = api_client::read_with_fallback(
        data_dir,
        |client| client.validate_output(execution_id, &request),
        || file_direct::validate_output(data_dir, execution_id, node, contract, value),
    )?;
    match response {
        WorkflowValidateOutputResult::Valid => {
            Ok(format!("ok: artifact schema '{contract}' is satisfied\n"))
        }
        WorkflowValidateOutputResult::Invalid { reason, details } => Err(CliError::InvalidInput(
            format!("artifact schema violation ({reason}): {details}"),
        )),
    }
}

pub(super) fn cmd_output_get(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node(node)?;
    let response = api_client::read_with_fallback(
        data_dir,
        |client| client.get_output(execution_id, node),
        || file_direct::get_output(data_dir, execution_id, node),
    )?;
    format_output(response, node, json)
}

fn format_output(
    response: WorkflowGetOutputResult,
    node: &str,
    json: bool,
) -> Result<String, CliError> {
    let view = OutputGetView::from(response);
    if json {
        let text = serde_json::to_string_pretty(&view)
            .map_err(|error| CliError::Other(format!("serialize output: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    match view {
        OutputGetView::Submitted {
            contract,
            artifact,
            submitted_at,
            request_id,
            timestamp,
        } => {
            let mut output = format!(
                "submitted: node={node} contract={}\n",
                contract.as_deref().unwrap_or("none")
            );
            if let Some(submitted_at) = submitted_at {
                output.push_str(&format!("submitted_at: {submitted_at}\n"));
            }
            if let Some(request_id) = request_id {
                output.push_str(&format!("request_id: {request_id}\n"));
            }
            output.push_str(&format!("timestamp: {timestamp}\n"));
            output.push_str(&format!(
                "artifact:\n{}\n",
                serde_json::to_string_pretty(&artifact)
                    .map_err(|error| CliError::Other(format!("serialize artifact: {error}")))?
            ));
            Ok(output)
        }
        OutputGetView::NotSubmitted => Ok(format!("not_submitted: node={node}\n")),
    }
}

fn validate_contract_argument(contract: &str) -> Result<(), CliError> {
    if contract.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--type must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn read_submit_input_json(
    json_arg: Option<String>,
    file_arg: Option<PathBuf>,
) -> Result<String, CliError> {
    match (json_arg, file_arg) {
        (Some(_), Some(_)) => Err(CliError::InvalidInput(
            "--json and --file are mutually exclusive".to_string(),
        )),
        (Some(raw), None) => Ok(raw),
        (None, Some(path)) => read_file(&path),
        (None, None) => Err(CliError::InvalidInput(
            "either --json or --file is required".to_string(),
        )),
    }
}

fn read_file(path: &Path) -> Result<String, CliError> {
    std::fs::read_to_string(path)
        .map_err(|error| CliError::InvalidInput(format!("Failed to read file {path:?}: {error}")))
}

fn parse_json_input(raw_json: String) -> Result<serde_json::Value, CliError> {
    serde_json::from_str(&raw_json)
        .map_err(|error| CliError::InvalidInput(format!("Failed to parse JSON: {error}")))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum OutputGetView {
    Submitted {
        contract: Option<String>,
        artifact: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

impl From<WorkflowGetOutputResult> for OutputGetView {
    fn from(result: WorkflowGetOutputResult) -> Self {
        match result {
            WorkflowGetOutputResult::Submitted {
                contract,
                structured_output,
                submitted_at,
                request_id,
                timestamp,
            } => Self::Submitted {
                contract,
                artifact: structured_output,
                submitted_at,
                request_id,
                timestamp,
            },
            WorkflowGetOutputResult::NotSubmitted => Self::NotSubmitted,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::super::common::test_support::{make_execution, test_uuid, write_execution_file};
    use super::super::Cli;
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
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
        WorkflowEventLog::new(data_dir)
            .append(&WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                permission_mode: "ask".to_string(),
                definition,
                timestamp: 1.0,
            })
            .unwrap();
    }

    #[test]
    fn output_commands_parse_with_node_and_type_vocabulary() {
        let execution_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec![
                "releash",
                "workflow",
                "output",
                "submit",
                execution_id,
                "--node",
                "review",
                "--type",
                "review-verdict",
                "--json",
                r#"{"verdict":"LGTM"}"#,
            ],
            vec![
                "releash",
                "workflow",
                "output",
                "validate",
                execution_id,
                "--node",
                "review",
                "--type",
                "review-verdict",
                "--file",
                "out.json",
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

        assert!(Cli::try_parse_from([
            "releash",
            "workflow",
            "output",
            "validate",
            execution_id,
            "--node",
            "review",
            "--file",
            "out.json",
        ])
        .is_err());
        let legacy_node_flag = ["--st", "ep"].concat();
        assert!(Cli::try_parse_from(vec![
            "releash".to_string(),
            "workflow".to_string(),
            "output".to_string(),
            "get".to_string(),
            execution_id.to_string(),
            legacy_node_flag,
            "review".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn submit_requires_running_app() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(10);
        seed_artifact_node(temp.path(), &execution_id);

        let error = cmd_output_submit(
            temp.path(),
            &execution_id,
            "review",
            Some("node-execution-review".to_string()),
            "review-verdict",
            Some(r#"{"verdict":"LGTM"}"#.to_string()),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(error, CliError::Other(message) if message.contains("アプリの起動が必要"))
        );
    }

    #[test]
    fn validate_uses_file_fallback_when_app_is_not_running() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(11);
        seed_artifact_node(temp.path(), &execution_id);
        let valid_file = temp.path().join("valid.json");
        std::fs::write(&valid_file, r#"{"verdict":"LGTM"}"#).unwrap();
        assert_eq!(
            cmd_output_validate(
                temp.path(),
                &execution_id,
                "review",
                "review-verdict",
                &valid_file,
            )
            .unwrap(),
            "ok: artifact schema 'review-verdict' is satisfied\n"
        );

        let invalid_file = temp.path().join("invalid.json");
        std::fs::write(&invalid_file, r#"{"missing":"verdict"}"#).unwrap();
        assert!(matches!(
            cmd_output_validate(
                temp.path(),
                &execution_id,
                "review",
                "review-verdict",
                &invalid_file,
            ),
            Err(CliError::InvalidInput(_))
        ));
    }

    #[test]
    fn get_reads_latest_artifact_through_file_fallback() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(12);
        seed_artifact_node(temp.path(), &execution_id);
        let log = WorkflowEventLog::new(temp.path());
        for (index, verdict) in ["FIX", "LGTM"].into_iter().enumerate() {
            log.append(&WorkflowEvent::ArtifactProduced {
                execution_id: execution_id.clone(),
                node_execution_id: format!("node-{index}"),
                node_name: "review".to_string(),
                contract: Some("review-verdict".to_string()),
                value: serde_json::json!({"verdict": verdict}),
                request_id: Some(format!("request-{index}")),
                submitted_at: Some(2.0 + index as f64),
                timestamp: 2.0 + index as f64,
            })
            .unwrap();
        }

        let output = cmd_output_get(temp.path(), &execution_id, "review", true).unwrap();
        let output: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["status"], "submitted");
        assert_eq!(output["artifact"]["verdict"], "LGTM");
        assert_eq!(output["request_id"], "request-1");
        assert!(output.get("structured_output").is_none());
    }

    #[test]
    fn get_reports_not_submitted_for_known_node() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(13);
        seed_artifact_node(temp.path(), &execution_id);

        assert_eq!(
            cmd_output_get(temp.path(), &execution_id, "review", false).unwrap(),
            "not_submitted: node=review\n"
        );
    }

    #[test]
    fn unknown_node_is_rejected_by_file_fallback() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(14);
        seed_artifact_node(temp.path(), &execution_id);
        assert!(matches!(
            cmd_output_get(temp.path(), &execution_id, "missing", true),
            Err(CliError::InvalidInput(_))
        ));
    }
}
