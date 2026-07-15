//! Releash アプリが起動していない場合だけ使う read-only fallback。
//!
//! mutation はここへ追加しない。read model は Tauri surface と同じ query/usecase を
//! composition し、CLI 固有の状態遷移を持たない。

use std::path::Path;

use super::common::{ensure_existing_data_dir, validate_execution_id, CliError};
use crate::adaptor::presenter::workflow::workflow_execution_to_view;
use crate::domain::workflow::{ExecutionStatusFilter, WorkflowError, WorkflowPageRequest};
use crate::usecase::workflow::dto::{WorkflowExecutionSummaryDto, WorkflowSummaryDto};
use crate::usecase::workflow::{
    WorkflowGetOutputResult, WorkflowReadUsecase, WorkflowValidateOutputResult,
};

pub(super) fn list_workflows(
    workflows_dir: &Path,
    data_dir: &Path,
) -> Result<Vec<WorkflowSummaryDto>, CliError> {
    ensure_existing_data_dir(data_dir)?;
    read_usecase(data_dir, Some(workflows_dir))?
        .list_workflow_summaries()
        .map_err(|error| CliError::Other(error.to_string()))
}

pub(super) fn list_executions(
    data_dir: &Path,
    status: Option<ExecutionStatusFilter>,
    worktree: Option<&str>,
) -> Result<Vec<WorkflowExecutionSummaryDto>, CliError> {
    ensure_existing_data_dir(data_dir)?;
    read_usecase(data_dir, None)?
        .list_executions_filtered(status, worktree, WorkflowPageRequest::new(0, usize::MAX))
        .map_err(workflow_error_to_cli_error)
}

pub(super) fn execution_status(
    data_dir: &Path,
    execution_id: &str,
) -> Result<crate::adaptor::protocol::workflow::WorkflowExecutionView, CliError> {
    ensure_existing_data_dir(data_dir)?;
    let read = read_usecase(data_dir, None)?;
    ensure_execution_exists(&read, execution_id)?;
    read.get_execution_state(execution_id)
        .map_err(|error| CliError::Other(error.to_string()))?
        .map(workflow_execution_to_view)
        .ok_or_else(|| {
            CliError::NotFound(format!("Workflow execution log not found: {execution_id}"))
        })
}

pub(super) fn execution_log(
    data_dir: &Path,
    execution_id: &str,
) -> Result<Vec<serde_json::Value>, CliError> {
    ensure_existing_data_dir(data_dir)?;
    read_usecase(data_dir, None)?
        .get_execution_log(execution_id)
        .map_err(workflow_error_to_cli_error)
}

pub(super) fn validate_output(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
    contract_name: &str,
    value: serde_json::Value,
) -> Result<WorkflowValidateOutputResult, CliError> {
    ensure_existing_data_dir(data_dir)?;
    read_usecase(data_dir, None)?
        .validate_output_for_contract(execution_id, node, contract_name, value)
        .map_err(workflow_error_to_cli_error)
}

pub(super) fn get_output(
    data_dir: &Path,
    execution_id: &str,
    node: &str,
) -> Result<WorkflowGetOutputResult, CliError> {
    ensure_existing_data_dir(data_dir)?;
    read_usecase(data_dir, None)?
        .get_output(execution_id, node)
        .map_err(workflow_error_to_cli_error)
}

fn read_usecase(
    data_dir: &Path,
    workflows_dir: Option<&Path>,
) -> Result<WorkflowReadUsecase, CliError> {
    crate::adaptor::controller::wiring::build_file_direct_workflow_read_usecase(
        data_dir.to_path_buf(),
        workflows_dir.map(Path::to_path_buf),
    )
    .map_err(CliError::Other)
}

fn ensure_execution_exists(read: &WorkflowReadUsecase, execution_id: &str) -> Result<(), CliError> {
    validate_execution_id(execution_id)?;
    if read
        .get_execution(execution_id)
        .map_err(|error| CliError::Other(error.to_string()))?
        .is_none()
    {
        return Err(CliError::NotFound(format!(
            "Workflow execution not found: {execution_id}"
        )));
    }
    Ok(())
}

fn workflow_error_to_cli_error(error: WorkflowError) -> CliError {
    match error {
        WorkflowError::NotFound(message) => CliError::NotFound(message),
        WorkflowError::Validation(message)
        | WorkflowError::InvalidState(message)
        | WorkflowError::UnauthorizedApprovalTarget(message) => CliError::InvalidInput(message),
        WorkflowError::External(message) => CliError::Other(message),
    }
}
