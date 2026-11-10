use crate::adaptor::presenter::error::AppError;
use std::sync::Arc;

use crate::adaptor::controller::client::workflow::validate_execution_id;
use crate::usecase::workflow::command::{AbortExecutionCommand, StartExecutionCommand};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub(crate) async fn start_workflow_shared(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    workflow_name: String,
    worktree_path: String,
    request: Option<String>,
) -> Result<String, AppError> {
    runtime
        .start_execution(StartExecutionCommand {
            workflow_name,
            worktree_path,
            request,
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
