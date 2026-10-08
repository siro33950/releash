use crate::adaptor::presenter::error::AppError;
use crate::adaptor::presenter::workflow_wire::WorkflowSubmitArtifactInput;
use crate::usecase::workflow::command::{SubmitOutputArtifact, SubmitOutputCommand};
use crate::usecase::workflow::WorkflowRuntimeUsecase;
use std::sync::Arc;

pub(crate) async fn workflow_submit_output_shared(
    runtime: &Arc<WorkflowRuntimeUsecase>,
    node_execution_id: String,
    artifact: Option<WorkflowSubmitArtifactInput>,
) -> Result<(), AppError> {
    runtime
        .submit_output(SubmitOutputCommand {
            node_execution_id,
            artifact: artifact.map(|artifact| SubmitOutputArtifact {
                contract: artifact.contract,
                value: artifact.value,
            }),
        })
        .await
        .map_err(AppError::from_failure)
}
