//! Structured-output validation and transactional mutation preparation.

use crate::domain::workflow::entities::workflow_execution::ExecutionTree as DomainExecutionTree;
use crate::domain::workflow::services::{contract as workflow_contract, secret_masker};
use crate::domain::workflow::WorkflowDefinition;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::{ContractType, ContractValidationResult};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

#[derive(Debug)]
pub(crate) struct ValidatedSubmissionOutput {
    pub(crate) artifact: serde_json::Value,
    pub(crate) result: Option<String>,
}

#[derive(Debug)]
pub(crate) struct SubmissionTargetContext {
    pub(crate) node_name: String,
    pub(crate) session_id: Option<String>,
    pub(crate) attempt: u32,
}

pub(crate) fn validate_submit_output_request(
    node_execution_id: &str,
) -> Result<(), WorkflowRuntimeError> {
    if node_execution_id.trim().is_empty() {
        return Err(WorkflowRuntimeError::ValidationError(
            "node_execution_id must not be empty".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_submit_target_context(
    exec: &DomainExecutionTree,
    execution_id: &str,
    node_execution_id: &str,
) -> Result<SubmissionTargetContext, WorkflowRuntimeError> {
    use crate::domain::workflow::entities::workflow_execution::NodeSubmitRejection;

    let target =
        exec.admit_node_submit(node_execution_id)
            .map_err(|rejection| match rejection {
                NodeSubmitRejection::ExecutionNotActive => {
                    WorkflowRuntimeError::InvalidState(format!(
                        "execution {execution_id} is not accepting node submit (state: {})",
                        exec.state().as_str()
                    ))
                }
                NodeSubmitRejection::NodeExecutionNotFound => {
                    WorkflowRuntimeError::ValidationError(format!(
                "node execution '{node_execution_id}' was not found in execution '{execution_id}'"
            ))
                }
                NodeSubmitRejection::AttemptNotCurrent => WorkflowRuntimeError::InvalidState(
                    format!("active node execution '{node_execution_id}' was not found"),
                ),
            })?;
    Ok(SubmissionTargetContext {
        node_name: target.node_name,
        session_id: target.session_id,
        attempt: target.attempt,
    })
}

pub(crate) fn validate_artifact_contract_for_workflow(
    workflow: &WorkflowDefinition,
    node_name: &str,
    contract: &str,
) -> Result<(), WorkflowRuntimeError> {
    ContractType::new(contract).map_err(|_| {
        WorkflowRuntimeError::ValidationError("contract must not be empty".to_string())
    })?;
    let expected_contract = workflow_contract::lookup_node_contract(workflow, node_name)
        .ok_or_else(|| {
            WorkflowRuntimeError::ValidationError(format!(
                "node '{node_name}' does not declare an Artifact contract"
            ))
        })?;
    if expected_contract != contract {
        return Err(WorkflowRuntimeError::ValidationError(format!(
            "contract mismatch: node '{node_name}' expects '{expected_contract}', got '{contract}'"
        )));
    }
    Ok(())
}

pub(crate) fn validate_submission_output_with_secrets(
    workflow: &WorkflowDefinition,
    contract: &str,
    artifact: serde_json::Value,
    secrets: &[String],
) -> Result<ValidatedSubmissionOutput, WorkflowRuntimeError> {
    let redacted = secret_masker::mask_sensitive_artifact(contract, artifact, secrets);
    match workflow_contract::validate_artifact_value(&workflow.schemas, contract, redacted.clone())
    {
        ContractValidationResult::Valid { artifact, result } => {
            Ok(ValidatedSubmissionOutput { artifact, result })
        }
        ContractValidationResult::Invalid(violation) => {
            Err(WorkflowRuntimeError::ValidationError(format!(
                "artifact schema validation failed ({}): {}",
                violation.reason, violation.details
            )))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn artifact_produced_event(
    execution_id: &str,
    node_execution_id: &str,
    node_name: &str,
    contract: String,
    artifact: serde_json::Value,
    request_id: Option<String>,
    submitted_at: Option<f64>,
    timestamp: f64,
) -> WorkflowEvent {
    WorkflowEvent::ArtifactProduced {
        execution_id: execution_id.to_string(),
        node_execution_id: node_execution_id.to_string(),
        node_name: node_name.to_string(),
        contract: Some(contract),
        value: artifact,
        request_id,
        submitted_at,
        timestamp,
    }
}

#[cfg(test)]
#[path = "output_submission_test.rs"]
mod output_submission_tests;
