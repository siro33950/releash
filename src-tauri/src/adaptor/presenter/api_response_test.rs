pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_実行開始応答_識別子とjson形式を保つ() {
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
