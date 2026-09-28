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
mod tests {
    use super::*;

    #[test]
    fn start_execution_response_converts_id_and_preserves_json_shape() {
        // Given
        let execution_id = "execution-1".to_string();
        // When
        let response = StartExecutionResponse::from(execution_id);
        // Then
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({"execution_id": "execution-1"})
        );
    }

    #[test]
    fn validation_and_get_responses_use_status_tags() {
        assert_eq!(
            serde_json::to_value(ValidateArtifactResponse::Valid).unwrap(),
            serde_json::json!({"status": "valid"})
        );
        assert_eq!(
            serde_json::to_value(GetArtifactResponse::NotSubmitted).unwrap(),
            serde_json::json!({"status": "not_submitted"})
        );
    }

    #[test]
    fn read_results_convert_to_wire_responses() {
        let validation = WorkflowValidateOutputResult::Invalid {
            reason: "schema_violation".to_string(),
            details: "missing status".to_string(),
        };
        assert_eq!(
            ValidateArtifactResponse::from(validation),
            ValidateArtifactResponse::Invalid {
                reason: "schema_violation".to_string(),
                details: "missing status".to_string(),
            }
        );

        let output = WorkflowGetOutputResult::Submitted {
            contract: Some("review-result".to_string()),
            structured_output: serde_json::json!({"status": "approved"}),
            submitted_at: Some(10.0),
            request_id: Some("request-1".to_string()),
            timestamp: 11.0,
        };
        assert_eq!(
            WorkflowGetOutputResult::from(GetArtifactResponse::from(output.clone())),
            output
        );
        let response = GetArtifactResponse::from(output);
        assert_eq!(
            response,
            GetArtifactResponse::Submitted {
                contract: Some("review-result".to_string()),
                value: serde_json::json!({"status": "approved"}),
                submitted_at: Some(10.0),
                request_id: Some("request-1".to_string()),
                timestamp: 11.0,
            }
        );
        let wire = serde_json::to_value(response).unwrap();
        assert_eq!(wire["value"], serde_json::json!({"status": "approved"}));
        assert!(wire.get("structured_output").is_none());
    }
}
