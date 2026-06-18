use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowAbortRunGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortRunCommand {
    pub run_id: String,
    pub expected_node_name: Option<String>,
}

#[derive(Clone)]
pub(crate) struct WorkflowAbortRunUsecase {
    runtime: Arc<dyn WorkflowAbortRunGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowAbortRunUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowAbortRunGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: AbortRunCommand) -> Result<(), WorkflowError> {
        self.preflight.validate_abort_run(&command)?;
        self.runtime.abort_run(command).await
    }
}
