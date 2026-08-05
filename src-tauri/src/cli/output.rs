use std::path::{Path, PathBuf};

use clap::Subcommand;

use super::api_client;
use super::common::{validate_execution_id, validate_node, CliError};
use super::file_direct;
use crate::adaptor::controller::api::protocol::SubmitArtifactRequest;
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
#[path = "output_test.rs"]
mod output_tests;
