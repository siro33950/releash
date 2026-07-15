use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowResumeExecutionGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeExecutionCommand {
    pub execution_id: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowResumeExecutionUsecase {
    runtime: Arc<dyn WorkflowResumeExecutionGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowResumeExecutionUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowResumeExecutionGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(
        &self,
        command: ResumeExecutionCommand,
    ) -> Result<(), WorkflowError> {
        self.preflight.validate_resume_execution(&command)?;
        self.runtime.resume_execution(command).await
    }
}
