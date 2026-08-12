//! Structured-output validation and transactional mutation preparation.

use std::collections::HashMap;

use crate::domain::workflow::entities::workflow_execution::WorkflowExecution as DomainWorkflowExecution;
use crate::domain::workflow::services::{contract as workflow_contract, secret_masker};
use crate::domain::workflow::RuntimeArtifact;
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
    exec: &DomainWorkflowExecution,
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

pub(crate) fn submitted_node_artifact_for(
    artifacts: &HashMap<String, RuntimeArtifact>,
    node_name: &str,
    attempt: u32,
    contract: &str,
) -> Option<RuntimeArtifact> {
    let output = artifacts.get(node_name)?;
    if output.attempt == attempt
        && output.contract.as_deref() == Some(contract)
        && output.artifact.is_some()
    {
        Some(output.clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{SchemaDef, TokenUsage};

    fn node_output(attempt: u32, contract: Option<&str>, structured: bool) -> RuntimeArtifact {
        RuntimeArtifact {
            node_name: "review-a".to_string(),
            attempt,
            session_id: None,
            result: Some("LGTM".to_string()),
            artifact: structured.then(|| serde_json::json!({"verdict": "LGTM"})),
            contract: contract.map(ToOwned::to_owned),
            token_usage: Some(TokenUsage::default()),
            completed_at: 1000.0,
        }
    }

    #[test]
    fn submitted_node_output_requires_matching_attempt_contract_and_artifact() {
        let outputs = HashMap::from([(
            "review-a".to_string(),
            node_output(2, Some("review-verdict"), true),
        )]);

        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "review-verdict").is_some());
        assert!(submitted_node_artifact_for(&outputs, "review-a", 1, "review-verdict").is_none());
        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "other-contract").is_none());

        let outputs = HashMap::from([(
            "review-a".to_string(),
            node_output(2, Some("review-verdict"), false),
        )]);
        assert!(submitted_node_artifact_for(&outputs, "review-a", 2, "review-verdict").is_none());
    }

    #[test]
    fn validate_submit_output_request_requires_node_execution_identity() {
        assert!(matches!(
            validate_submit_output_request(""),
            Err(WorkflowRuntimeError::ValidationError(message))
                if message == "node_execution_id must not be empty"
        ));
        assert!(validate_submit_output_request("node-execution-1").is_ok());
    }

    #[test]
    fn validate_submission_output_with_secrets_accepts_schema_valid_artifact() {
        let workflow = WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: [(
                "spec-directory".to_string(),
                SchemaDef::Object {
                    properties: [
                        ("spec_dir".to_string(), SchemaDef::String { r#enum: None }),
                        ("design".to_string(), SchemaDef::String { r#enum: None }),
                    ]
                    .into_iter()
                    .collect(),
                    required: ["spec_dir".to_string(), "design".to_string()]
                        .into_iter()
                        .collect(),
                },
            )]
            .into_iter()
            .collect(),
            nodes: vec![],
        };
        let validated = validate_submission_output_with_secrets(
            &workflow,
            "spec-directory",
            serde_json::json!({
                "spec_dir": "docs/specs/feat-token",
                "design": "design.md"
            }),
            &["SECRET_TOKEN".to_string()],
        )
        .unwrap();

        assert_eq!(validated.artifact["spec_dir"], "docs/specs/feat-token");
    }

    #[test]
    fn artifact_produced_event_preserves_external_shape() {
        let event = artifact_produced_event(
            "execution-1",
            "node-execution-1",
            "review",
            "review-verdict".to_string(),
            serde_json::json!({"verdict": "LGTM"}),
            Some("request-1".to_string()),
            Some(10.0),
            20.0,
        );

        assert!(matches!(
            event,
            WorkflowEvent::ArtifactProduced {
                execution_id,
                node_name,
                contract,
                request_id: Some(request_id),
                submitted_at: Some(submitted_at),
                timestamp,
                ..
            } if execution_id == "execution-1"
                && node_name == "review"
                && contract.as_deref() == Some("review-verdict")
                && request_id == "request-1"
                && (submitted_at - 10.0).abs() < f64::EPSILON
                && (timestamp - 20.0).abs() < f64::EPSILON
        ));
    }
}
