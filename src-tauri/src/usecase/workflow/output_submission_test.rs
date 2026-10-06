pub(crate) mod tests {
    use super::super::*;
    use crate::domain::workflow::SchemaDef;

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
            entry: "main".to_string(),
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
