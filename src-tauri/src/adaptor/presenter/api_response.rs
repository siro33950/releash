use crate::usecase::workflow::{WorkflowGetOutputResult, WorkflowValidateOutputResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StartExecutionResponse {
    pub(crate) execution_id: String,
}

impl From<String> for StartExecutionResponse {
    fn from(execution_id: String) -> Self {
        Self { execution_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MutationResponse {
    pub(crate) ok: bool,
}

impl MutationResponse {
    pub(crate) fn ok() -> Self {
        Self { ok: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ValidateArtifactResponse {
    Valid,
    Invalid { reason: String, details: String },
}

impl From<WorkflowValidateOutputResult> for ValidateArtifactResponse {
    fn from(result: WorkflowValidateOutputResult) -> Self {
        match result {
            WorkflowValidateOutputResult::Valid => Self::Valid,
            WorkflowValidateOutputResult::Invalid { reason, details } => {
                Self::Invalid { reason, details }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum GetArtifactResponse {
    Submitted {
        contract: Option<String>,
        value: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        submitted_at: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

impl From<WorkflowGetOutputResult> for GetArtifactResponse {
    fn from(result: WorkflowGetOutputResult) -> Self {
        match result {
            WorkflowGetOutputResult::Submitted {
                contract,
                structured_output,
                submitted_at,
                request_id,
                timestamp,
            } => Self::Submitted {
                contract,
                value: structured_output,
                submitted_at,
                request_id,
                timestamp,
            },
            WorkflowGetOutputResult::NotSubmitted => Self::NotSubmitted,
        }
    }
}

impl From<GetArtifactResponse> for WorkflowGetOutputResult {
    fn from(response: GetArtifactResponse) -> Self {
        match response {
            GetArtifactResponse::Submitted {
                contract,
                value,
                submitted_at,
                request_id,
                timestamp,
            } => Self::Submitted {
                contract,
                structured_output: value,
                submitted_at,
                request_id,
                timestamp,
            },
            GetArtifactResponse::NotSubmitted => Self::NotSubmitted,
        }
    }
}

#[cfg(test)]
#[path = "api_response_test.rs"]
mod api_response_tests;
