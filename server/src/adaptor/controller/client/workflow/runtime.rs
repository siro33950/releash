use crate::adaptor::presenter::error::AppError;
use std::sync::Arc;

use crate::adaptor::controller::client::workflow::validate_execution_id;
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, StartExecutionCommand,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

fn parse_execution_origin(
    value: Option<String>,
) -> Result<crate::domain::workflow::ExecutionOrigin, AppError> {
    value
        .as_deref()
        .map(crate::domain::workflow::ExecutionOrigin::from_public_value)
        .unwrap_or(Ok(crate::domain::workflow::ExecutionOrigin::DesktopUi))
        .map_err(AppError::from_failure)
}

pub(crate) async fn start_workflow_shared(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    workflow_name: String,
    worktree_path: String,
    request: Option<String>,
    created_from: Option<String>,
) -> Result<String, AppError> {
    let created_from = parse_execution_origin(created_from)?;
    runtime
        .start_execution(StartExecutionCommand {
            workflow_name,
            worktree_path,
            request,
            created_from,
        })
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn abort_workflow_shared(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    execution_id: String,
) -> Result<(), AppError> {
    validate_execution_id(&execution_id)?;
    runtime
        .abort_execution(AbortExecutionCommand {
            execution_id,
            expected_node_name: None,
        })
        .await
        .map_err(|e| {
            log::error!("abort_workflow failed: code=ABORT_WORKFLOW_FAILED");
            AppError::from_failure(e)
        })
}

pub(crate) async fn approve_workflow_node_shared(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    command: ApprovalCommand,
) -> Result<(), AppError> {
    validate_execution_id(&command.execution_id)?;
    runtime
        .resolve_approval(command)
        .await
        .map_err(AppError::from_failure)
}

#[cfg(test)]
#[path = "runtime_test.rs"]
mod runtime_tests;
