use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowStopExecutionGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopExecutionCommand {
    pub execution_id: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowStopExecutionUsecase {
    runtime: Arc<dyn WorkflowStopExecutionGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowStopExecutionUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowStopExecutionGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: StopExecutionCommand) -> Result<(), WorkflowError> {
        self.preflight.validate_stop_execution(&command)?;
        self.runtime.stop_execution(command).await
    }
}
