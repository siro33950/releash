use std::sync::Arc;

use crate::domain::workflow::{TriggerSource, WorkflowDefinition, WorkflowError};
use crate::usecase::workflow::ports::WorkflowStartRunGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRunCommand {
    pub workflow_file_stem: String,
    pub worktree_path: String,
    pub task: Option<String>,
    pub trigger_source: TriggerSource,
    pub permission_mode: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStartRunCommand {
    pub workflow_file_stem: String,
    pub workflow: WorkflowDefinition,
    pub worktree_path: String,
    pub task: Option<String>,
    pub trigger_source: TriggerSource,
    pub permission_mode: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowStartRunUsecase {
    runtime: Arc<dyn WorkflowStartRunGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowStartRunUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowStartRunGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: StartRunCommand) -> Result<String, WorkflowError> {
        self.preflight.validate_start_run(&command)?;
        let worktree_path = self
            .runtime
            .resolve_start_run_worktree(command.worktree_path)
            .await?;
        let workflow = self
            .runtime
            .resolve_start_run_workflow(&command.workflow_file_stem)
            .await?;
        self.runtime
            .start_resolved_run(ResolvedStartRunCommand {
                workflow_file_stem: command.workflow_file_stem,
                workflow,
                worktree_path,
                task: command.task,
                trigger_source: command.trigger_source,
                permission_mode: command.permission_mode,
            })
            .await
    }
}
