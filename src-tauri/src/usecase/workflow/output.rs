use std::sync::Arc;

use serde_json::Value;

use crate::domain::workflow::{
    contract, secret_masker, ContractValidationResult, SecretSourceGateway, WorkflowError,
};

use super::event_draft;
use super::ports::WorkflowEventDraft;
use super::query_service::{WorkflowGetOutputResult, WorkflowQueryService};

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowValidateOutputResult {
    Valid,
    Invalid { reason: String, details: String },
}

#[derive(Clone)]
pub struct WorkflowOutputUsecase {
    query: WorkflowQueryService,
    secrets: Arc<dyn SecretSourceGateway>,
}

impl WorkflowOutputUsecase {
    pub fn new(query: WorkflowQueryService, secrets: Arc<dyn SecretSourceGateway>) -> Self {
        Self { query, secrets }
    }

    pub async fn validate_output_for_contract(
        &self,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        let events = self.query.read_events(execution_id).await?;
        let context = resolve_node_artifact_schema_from_drafts(&events, node_name, execution_id)?;
        if context.contract != contract {
            return Err(WorkflowError::validation(format!(
                "node '{node_name}' expects contract '{}', but '{contract}' was provided",
                context.contract
            )));
        }
        self.validate_with_context(context, structured_output)
    }

    fn validate_with_context(
        &self,
        context: event_draft::ArtifactSchemaContext,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        let secrets = self.secrets.configured_secret_values()?;
        let redacted =
            secret_masker::mask_sensitive_artifact(&context.contract, structured_output, &secrets);
        Ok(
            match contract::validate_artifact_value(&context.schemas, &context.contract, redacted) {
                ContractValidationResult::Valid { .. } => WorkflowValidateOutputResult::Valid,
                ContractValidationResult::Invalid(violation) => {
                    WorkflowValidateOutputResult::Invalid {
                        reason: violation.reason,
                        details: violation.details,
                    }
                }
            },
        )
    }

    pub async fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        let events = self.query.read_events(execution_id).await?;
        match event_draft::node_exists_in_drafts(&events, node_name, execution_id)
            .map_err(contract_lookup_error_to_workflow_error)?
        {
            true => {}
            false => {
                return Err(WorkflowError::validation(format!(
                    "node '{node_name}' is not defined in workflow execution '{execution_id}'"
                )))
            }
        }
        if event_draft::node_is_isolated_in_drafts(&events, node_name, execution_id)
            .map_err(contract_lookup_error_to_workflow_error)?
        {
            if let Some(submitted) =
                event_draft::latest_isolated_artifact_from_drafts(&events, node_name, execution_id)?
            {
                return Ok(WorkflowGetOutputResult::Submitted {
                    contract: submitted.contract,
                    structured_output: submitted.value,
                    submitted_at: submitted.submitted_at,
                    request_id: submitted.request_id,
                    timestamp: submitted.timestamp,
                });
            }
            return self
                .query
                .get_node_output_from_events(execution_id, node_name, &events);
        }

        Ok(WorkflowQueryService::get_output_from_events(
            &events, node_name,
        ))
    }
}

fn resolve_node_artifact_schema_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
    execution_id: &str,
) -> Result<event_draft::ArtifactSchemaContext, WorkflowError> {
    event_draft::resolve_node_artifact_schema_from_drafts(events, node_name, execution_id)
        .map_err(contract_lookup_error_to_workflow_error)
}

fn contract_lookup_error_to_workflow_error(error: contract::ContractLookupError) -> WorkflowError {
    match error {
        contract::ContractLookupError::ExecutionNotFound { execution_id } => {
            WorkflowError::NotFound(format!("Workflow execution not found: {execution_id}"))
        }
        contract::ContractLookupError::InvalidExecutionStartedPayload { details } => {
            WorkflowError::validation(details)
        }
        contract::ContractLookupError::NoArtifactContract {
            workflow_name,
            node,
        } => WorkflowError::validation(format!(
            "node '{node}' has no artifact in workflow '{workflow_name}'"
        )),
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
pub(crate) mod output_tests;
