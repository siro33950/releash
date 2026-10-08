pub(crate) mod abort_execution;
pub(crate) mod approval;
mod preflight;
pub(crate) mod retry_node;
pub(crate) mod start_execution;
pub(crate) mod submit_output;

pub use abort_execution::AbortExecutionCommand;
pub(crate) use abort_execution::WorkflowAbortExecutionUsecase;
pub use approval::ApprovalCommand;
pub(crate) use preflight::WorkflowRuntimeCommandPreflight;
pub(crate) use retry_node::WorkflowRetryNodeUsecase;
pub use retry_node::{ResumeSessionNodeCommand, RetryNodeCommand};
pub(crate) use start_execution::WorkflowStartExecutionUsecase;
pub use start_execution::{ResolvedStartExecutionCommand, StartExecutionCommand};
pub(crate) use submit_output::WorkflowSubmitOutputUsecase;
pub use submit_output::{SubmitOutputArtifact, SubmitOutputCommand};

pub(crate) async fn retry_control_plane_conflicts<T, F, Fut>(
    retrying: &crate::usecase::retry::Retrying,
    target: &str,
    operation: F,
) -> Result<T, crate::domain::workflow::WorkflowError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, crate::domain::workflow::WorkflowError>>,
{
    retry_control_plane_operation(retrying, target, operation).await
}

pub(crate) async fn retry_control_plane_operation<T, E, F, Fut>(
    retrying: &crate::usecase::retry::Retrying,
    target: &str,
    mut operation: F,
) -> Result<T, E>
where
    E: crate::usecase::failure::RetryFailure,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    retrying
        .restart(
            crate::usecase::failure::FailureKey::new("workflow_control_plane", target),
            crate::common::retry::RetryBackoff::CONFLICT,
            |_| operation(),
        )
        .await
}

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;
