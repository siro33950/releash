use std::path::{Path, PathBuf};

use clap::Subcommand;

use super::common::{validate_execution_id, CliError};
use super::workflow_io;
use crate::domain::workflow::services::spec_directory as workflow_spec_directory;
use crate::domain::workflow::value_objects::ContractViolation;
use crate::domain::workflow::{contract, secret_masker, ContractValidationResult};
use crate::usecase::workflow::event_draft;
use crate::usecase::workflow::ports::WorkflowEventDraft;
use crate::usecase::workflow::WorkflowGetOutputResult;

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
    validate_node_argument(node)?;
    validate_contract_argument(contract)?;
    let raw_json = read_submit_input_json(json_arg, file_arg)?;
    let artifact: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|error| CliError::InvalidInput(format!("Failed to parse JSON: {error}")))?;

    let context = resolve_node_artifact_schema_via_log(data_dir, execution_id, node)?;
    if context.contract != contract {
        return Err(CliError::InvalidInput(format!(
            "contract mismatch: node '{node}' expects '{}', got '{contract}'",
            context.contract
        )));
    }
    if let ContractValidationResult::Invalid(violation) =
        validate_cli_artifact_output(&context, artifact.clone())
    {
        return Err(CliError::InvalidInput(format!(
            "artifact schema violation ({}): {}",
            violation.reason, violation.details
        )));
    }

    let output = workflow_io::enqueue_pending_command(
        data_dir,
        execution_id,
        workflow_io::CliRequestPayload::SubmitOutput {
            node_name: node.to_string(),
            node_execution_id: workflow_io::resolve_node_execution_id(node_execution),
            contract: contract.to_string(),
            artifact,
        },
    )?;
    Ok(format!("{}\n", output.format_stdout_line()))
}

pub(super) fn cmd_output_validate(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    file: &Path,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node_argument(node)?;
    let context = resolve_node_artifact_schema_via_log(data_dir, execution_id, node)?;
    let raw_json = std::fs::read_to_string(file).map_err(|error| {
        CliError::InvalidInput(format!("Failed to read file {file:?}: {error}"))
    })?;
    let artifact: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|error| CliError::InvalidInput(format!("Failed to parse JSON: {error}")))?;
    match validate_cli_artifact_output(&context, artifact) {
        ContractValidationResult::Valid { .. } => Ok(format!(
            "ok: artifact schema '{}' is satisfied\n",
            context.contract
        )),
        ContractValidationResult::Invalid(violation) => Err(CliError::InvalidInput(format!(
            "artifact schema violation ({}): {}",
            violation.reason, violation.details
        ))),
    }
}

pub(super) fn cmd_output_get(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node_argument(node)?;
    workflow_io::ensure_execution_exists(data_dir, execution_id)?;
    let events = workflow_io::read_execution_events(data_dir, execution_id)?;
    ensure_node_exists_via_log(&events, execution_id, node)?;
    let result = workflow_io::file_direct_query_service(data_dir)
        .get_output(execution_id, node)
        .map_err(|error| CliError::Other(error.to_string()))?;
    let view = OutputGetView::from(result);
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

fn validate_node_argument(node: &str) -> Result<(), CliError> {
    if node.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "--node must not be empty".to_string(),
        ));
    }
    Ok(())
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
        (None, Some(path)) => std::fs::read_to_string(&path).map_err(|error| {
            CliError::InvalidInput(format!("Failed to read file {path:?}: {error}"))
        }),
        (None, None) => Err(CliError::InvalidInput(
            "either --json or --file is required".to_string(),
        )),
    }
}

fn ensure_node_exists_via_log(
    events: &[WorkflowEventDraft],
    execution_id: &str,
    node: &str,
) -> Result<(), CliError> {
    match event_draft::node_exists_in_drafts(events, node, execution_id) {
        Ok(true) => Ok(()),
        Ok(false) => Err(CliError::InvalidInput(format!(
            "node '{node}' is not defined in workflow execution '{execution_id}'"
        ))),
        Err(contract::ContractLookupError::ExecutionNotFound { execution_id }) => Err(
            CliError::NotFound(format!("Workflow execution not found: {execution_id}")),
        ),
        Err(contract::ContractLookupError::InvalidExecutionStartedPayload { details }) => {
            Err(CliError::InvalidInput(details))
        }
        Err(contract::ContractLookupError::NoArtifactContract { .. }) => {
            Err(CliError::InvalidInput(format!(
                "node '{node}' is not defined in workflow execution '{execution_id}'"
            )))
        }
    }
}

fn resolve_node_artifact_schema_via_log(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
) -> Result<event_draft::ArtifactSchemaContext, CliError> {
    let events = workflow_io::read_execution_events(data_dir, execution_id)?;
    event_draft::resolve_node_artifact_schema_from_drafts(&events, node, execution_id).map_err(
        |error| match error {
            contract::ContractLookupError::ExecutionNotFound { .. } => {
                CliError::NotFound(format!("Workflow execution not found: {execution_id}"))
            }
            contract::ContractLookupError::InvalidExecutionStartedPayload { details } => {
                CliError::InvalidInput(details)
            }
            contract::ContractLookupError::NoArtifactContract {
                workflow_name,
                node,
            } => CliError::InvalidInput(format!(
                "node '{node}' has no artifact in workflow '{workflow_name}'"
            )),
        },
    )
}

fn validate_cli_artifact_output(
    context: &event_draft::ArtifactSchemaContext,
    artifact: serde_json::Value,
) -> ContractValidationResult {
    let redacted = secret_masker::mask_sensitive_artifact(&context.contract, artifact, &[]);
    match contract::validate_artifact_value(&context.schemas, &context.contract, redacted) {
        ContractValidationResult::Valid { artifact, result } => {
            let violations =
                workflow_spec_directory::validate_contract_value(&context.contract, &artifact);
            if violations.is_empty() {
                ContractValidationResult::Valid { artifact, result }
            } else {
                ContractValidationResult::Invalid(ContractViolation {
                    reason: "schema_violation".to_string(),
                    details: contract::format_schema_violations(&violations),
                })
            }
        }
        invalid => invalid,
    }
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
    use crate::adaptor::gateway::workflow::pending_command::{
        PendingCommandPayload, PendingCommandStore,
    };
    use crate::adaptor::gateway::workflow::schema::{
        NodeDefinition, NodeKind, SchemaDef, SessionSpec, Workflow,
    };
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};
    use clap::Parser;
    use tempfile::TempDir;

    fn seed_artifact_node(data_dir: &Path, execution_id: &str) {
        write_execution_file(
            data_dir,
            &make_execution(execution_id, "/repo", ExecutionStatus::Running, 1.0),
        );
        let definition = Workflow {
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
                    additional_properties: false,
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
                definition,
                timestamp: 1.0,
            })
            .unwrap();
    }

    #[test]
    fn output_commands_parse_with_node_flag_only() {
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

        let legacy_flag = ["--st", "ep"].concat();
        let argv = vec![
            "releash".to_string(),
            "workflow".to_string(),
            "output".to_string(),
            "get".to_string(),
            execution_id.to_string(),
            legacy_flag,
            "review".to_string(),
        ];
        assert!(Cli::try_parse_from(argv).is_err());
    }

    #[test]
    fn submit_validates_and_enqueues_node_artifact() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(10);
        seed_artifact_node(temp.path(), &execution_id);

        let output = cmd_output_submit(
            temp.path(),
            &execution_id,
            "review",
            Some("node-execution-review".to_string()),
            "review-verdict",
            Some(r#"{"verdict":"LGTM"}"#.to_string()),
            None,
        )
        .unwrap();
        assert!(output.starts_with(&format!("queued: execution_id={execution_id}")));

        let entries = PendingCommandStore::new(temp.path())
            .list_pending()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command.execution_id, execution_id);
        assert_eq!(
            entries[0].command.payload,
            PendingCommandPayload::SubmitOutput {
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-review".to_string()),
                contract: "review-verdict".to_string(),
                artifact: serde_json::json!({"verdict": "LGTM"}),
            }
        );
    }

    #[test]
    fn validate_rejects_invalid_artifact_without_writing_pending_command() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(11);
        seed_artifact_node(temp.path(), &execution_id);
        let file = temp.path().join("artifact.json");
        std::fs::write(&file, r#"{"missing":"verdict"}"#).unwrap();

        let error = cmd_output_validate(temp.path(), &execution_id, "review", &file).unwrap_err();
        assert!(matches!(error, CliError::InvalidInput(_)));
        assert!(PendingCommandStore::new(temp.path())
            .list_pending()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn get_reads_latest_artifact_through_query_service() {
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

        let human = cmd_output_get(temp.path(), &execution_id, "review", false).unwrap();
        assert!(human.starts_with("submitted: node=review"));
        assert!(human.contains("artifact:"));
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
    fn unknown_node_is_rejected() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(14);
        seed_artifact_node(temp.path(), &execution_id);
        assert!(matches!(
            cmd_output_get(temp.path(), &execution_id, "missing", true),
            Err(CliError::InvalidInput(_))
        ));
    }
}
