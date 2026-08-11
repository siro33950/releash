use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase,
};

use super::preflight::WorkflowRuntimeCommandPreflight;

#[derive(Debug, Clone, PartialEq)]
pub struct SubmitOutputArtifact {
    pub contract: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubmitOutputCommand {
    pub execution_id: String,
    pub node_name: String,
    pub node_execution_id: String,
    pub artifact: Option<SubmitOutputArtifact>,
}

#[derive(Clone)]
pub(crate) struct WorkflowSubmitOutputUsecase {
    control_plane: WorkflowControlPlaneUsecase,
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowSubmitOutputUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowControlPlaneGateway>) -> Self {
        Self {
            control_plane: WorkflowControlPlaneUsecase::new(runtime),
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub(crate) async fn execute(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.preflight.validate_submit_output(&command)?;
        self.control_plane.submit_output(command).await
    }
}
