use std::sync::Arc;

use crate::domain::workflow::{ExecutionTreeId, WorkflowError};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryNodeCommand {
    pub execution_id: String,
    pub node_execution_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSessionNodeCommand {
    pub execution_id: String,
    pub node_execution_id: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowRetryNodeUsecase {
    control_plane: WorkflowControlPlaneUsecase,
}

impl WorkflowRetryNodeUsecase {
    pub(crate) fn new(
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        runtime: Arc<dyn WorkflowControlPlaneGateway>,
    ) -> Self {
        Self {
            control_plane: WorkflowControlPlaneUsecase::new(queue, runtime),
        }
    }

    pub(crate) async fn execute(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        ExecutionTreeId::new(command.execution_id.clone())?;
        if command.node_execution_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "node_execution_id must not be empty",
            ));
        }
        self.control_plane.retry_node(command).await
    }
}
