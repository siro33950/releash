use std::sync::Arc;

use crate::domain::workflow::{ExecutionOrigin, WorkflowDefinition, WorkflowError};
use crate::usecase::workflow::ports::WorkflowStartExecutionGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartExecutionCommand {
    pub workflow_file_stem: String,
    pub worktree_path: String,
    pub request: Option<String>,
    pub created_from: ExecutionOrigin,
    pub permission_mode: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStartExecutionCommand {
    pub workflow_file_stem: String,
    pub workflow: WorkflowDefinition,
    pub worktree_path: String,
    pub request: Option<String>,
    pub created_from: ExecutionOrigin,
    pub permission_mode: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowStartExecutionUsecase {
    runtime: Arc<dyn WorkflowStartExecutionGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowStartExecutionUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowStartExecutionGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(
        &self,
        command: StartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        self.preflight.validate_start_execution(&command)?;
        let worktree_path = self
            .runtime
            .resolve_start_execution_worktree(command.worktree_path)
            .await?;
        let workflow = self
            .runtime
            .resolve_start_execution_workflow(&command.workflow_file_stem)
            .await?;
        self.runtime
            .start_resolved_execution(ResolvedStartExecutionCommand {
                workflow_file_stem: command.workflow_file_stem,
                workflow,
                worktree_path,
                request: command.request,
                created_from: command.created_from,
                permission_mode: command.permission_mode,
            })
            .await
    }
}
