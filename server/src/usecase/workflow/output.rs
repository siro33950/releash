use crate::domain::workflow::{contract, WorkflowError};

use super::event_draft;
use super::query_service::{WorkflowGetOutputResult, WorkflowQueryService};

#[derive(Clone)]
pub struct WorkflowOutputUsecase {
    query: WorkflowQueryService,
}

impl WorkflowOutputUsecase {
    pub fn new(query: WorkflowQueryService) -> Self {
        Self { query }
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

fn contract_lookup_error_to_workflow_error(error: contract::ContractLookupError) -> WorkflowError {
    match error {
        contract::ContractLookupError::ExecutionNotFound { execution_id } => {
            WorkflowError::NotFound(format!("Workflow execution not found: {execution_id}"))
        }
        contract::ContractLookupError::InvalidExecutionStartedPayload { details } => {
            WorkflowError::validation(details)
        }
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
pub(crate) mod output_tests;
