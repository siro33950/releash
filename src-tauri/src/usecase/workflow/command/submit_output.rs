use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowSubmitOutputGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq)]
pub struct SubmitOutputCommand {
    pub run_id: String,
    pub step_name: String,
    pub node_execution_id: Option<String>,
    pub contract: String,
    pub structured_output: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct WorkflowSubmitOutputUsecase {
    runtime: Arc<dyn WorkflowSubmitOutputGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowSubmitOutputUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowSubmitOutputGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.preflight.validate_submit_output(&command)?;
        self.runtime.submit_output(command).await
    }
}
