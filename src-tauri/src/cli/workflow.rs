use std::path::Path;

use clap::Subcommand;

use super::api_client;
use super::common::{validate_execution_id, CliError};
use super::file_direct;
use super::output::OutputSubcommand;
use crate::adaptor::protocol::workflow::{ExecutionStatusView, WorkflowExecutionView};

/// `releash workflow` の Agent-facing command / query 集合。
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
    /// 指定 execution の現在 read model を表示する。
    Status {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// node の Artifact に対する typed CLI 入口。
    Output {
        #[command(subcommand)]
        command: OutputSubcommand,
    },
}

pub(super) fn cmd_status(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    let execution = api_client::read_with_fallback(
        data_dir,
        |client| client.execution_status(execution_id),
        || file_direct::execution_status(data_dir, execution_id),
    )?;
    format_execution_status(execution, json)
}

fn format_execution_status(
    execution: WorkflowExecutionView,
    json: bool,
) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&execution)
            .map_err(|error| CliError::Other(format!("serialize execution: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let current_node = execution.current_node.as_deref().unwrap_or("");
    Ok(format!(
        "execution_id:  {}\nworkflow:      {}\nstatus:        {}\ncurrent_node:  {}\nupdated_at:    {}\ninput_tokens:  {}\noutput_tokens: {}\n",
        execution.id,
        execution.workflow_name,
        execution_status_name(execution.status),
        current_node,
        execution.updated_at,
        execution.total_token_usage.input_tokens,
        execution.total_token_usage.output_tokens,
    ))
}

fn execution_status_name(status: ExecutionStatusView) -> &'static str {
    match status {
        ExecutionStatusView::Running => "running",
        ExecutionStatusView::WaitingApproval => "waiting_approval",
        ExecutionStatusView::Interrupted => "interrupted",
        ExecutionStatusView::Completed => "completed",
        ExecutionStatusView::Failed => "failed",
        ExecutionStatusView::Aborted => "aborted",
    }
}

#[cfg(test)]
#[path = "workflow_test.rs"]
mod workflow_tests;
