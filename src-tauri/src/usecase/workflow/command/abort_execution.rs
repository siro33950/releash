use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowAbortExecutionGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortExecutionCommand {
    pub execution_id: String,
    pub expected_node_name: Option<String>,
}

#[derive(Clone)]
pub(crate) struct WorkflowAbortExecutionUsecase {
    runtime: Arc<dyn WorkflowAbortExecutionGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowAbortExecutionUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowAbortExecutionGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(
        &self,
        command: AbortExecutionCommand,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_abort_execution(&command)?;
        self.runtime.abort_execution(command).await
    }
}
