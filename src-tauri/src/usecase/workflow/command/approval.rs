use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::ports::WorkflowApprovalGateway;

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCommand {
    pub execution_id: String,
    pub node_name: String,
    pub node_execution_id: Option<String>,
    pub comment: Option<String>,
}

#[derive(Clone)]
pub(crate) struct WorkflowApprovalUsecase {
    runtime: Arc<dyn WorkflowApprovalGateway>,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowApprovalUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowApprovalGateway>) -> Self {
        Self {
            runtime,
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.preflight.validate_approval(&command)?;
        self.runtime.resolve_approval(command).await
    }
}
