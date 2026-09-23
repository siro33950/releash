use super::test_helpers::Fixture;
use super::*;
use crate::adaptor::gateway::app_config::AppConfig;
use crate::adaptor::gateway::workflow::{
    WorkflowRuntimeCommandGateway, WorkflowSecretSourceConfigGateway,
};
use crate::domain::app_config::ConfigSecretRepository;
use crate::domain::workflow::{NodeFact, SecretSourceGateway};
use crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand;
use crate::usecase::workflow::command::{
    ApprovalCommand, SubmitOutputArtifact, SubmitOutputCommand,
};
use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;

#[tokio::test]
async fn test_workflowの秘匿_設定取得失敗でも表示とartifactと承認でnotionを秘匿する() {
    for parse_failure in [false, true] {
        // Given
        let mut fixture = Fixture::new(0);
        let path = fixture._directory.path().join("releash.toml");
        if parse_failure {
            std::fs::write(&path, "[server]\ntoken = 'legacy-sensitive-value' invalid").unwrap();
        } else {
            std::fs::create_dir(&path).unwrap();
        }
        let config =
            toml::from_str("[notion.'/repo']\napi_token = 'notion-value-1234'\ndatabase_id = 'db'")
                .unwrap();
        let config: Arc<dyn ConfigSecretRepository> = Arc::new(AppConfig::new(config, path));
        fixture.app.secrets = Some(config.clone());
        let token = "notion-value-1234";

        // When
        let secrets = secret_source::collect_configured_secret_values(&fixture.app);
        let display_command =
            workflow_secret_masker::mask_sensitive_text(&format!("echo {token}"), &secrets);
        let artifact = build_command_artifact(
            &BTreeMap::new(),
            None,
            CommandRunOutput {
                exit_code: 0,
                stdout: token.into(),
                stderr: token.into(),
                duration_ms: 1,
            },
            &secrets,
        );

        // Then
        assert_eq!(display_command, "echo [REDACTED]");
        assert_eq!(artifact.value["stdout"], "[REDACTED]");
        assert_eq!(artifact.value["stderr"], "[REDACTED]");
        assert!(WorkflowSecretSourceConfigGateway::new(config)
            .configured_secret_values()
            .unwrap()
            .contains(&token.to_string()));

        // Given
        let execution_id = fixture
            .start(
                "  main:
    session: {provider: codex, facets: {instruction: policy-confirmation}}
    completion: {require: approval}
    artifact: result
schemas:
  result:
    type: object
    properties:
      value: {type: string}
    required: [value]",
            )
            .await;
        let snapshot = fixture
            .host
            .get_state_by_execution_id(&fixture.app, &execution_id)
            .await
            .unwrap();
        let node = snapshot
            .node_executions
            .iter()
            .find(|node| node.node_name == "main")
            .unwrap();
        let control = WorkflowControlPlaneUsecase::new(Arc::new(
            WorkflowRuntimeCommandGateway::new_with_driver(
                fixture.app.clone(),
                Arc::new(fixture.host.clone()),
            ),
        ));

        // When
        control
            .submit_output(SubmitOutputCommand {
                node_execution_id: node.id.clone(),
                artifact: Some(SubmitOutputArtifact {
                    contract: "result".into(),
                    value: serde_json::json!({"value": token}),
                }),
            })
            .await
            .unwrap();
        control
            .record_provider_stop(
                ProviderExecutionTreeStopCommand {
                    agent_session_id: node.session_id.clone().unwrap(),
                    tree_id: execution_id.clone(),
                    node_execution_id: node.id.clone(),
                    binding_id: "binding-redaction-test".into(),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        control
            .resolve_approval(ApprovalCommand {
                execution_id: execution_id.clone(),
                node_name: node.node_name.clone(),
                node_execution_id: Some(node.id.clone()),
                comment: Some(token.into()),
            })
            .await
            .unwrap();

        // Then
        let records = workflow_fact_log::read_tree_records(&fixture.store, &execution_id).unwrap();
        let artifact = records
            .iter()
            .find_map(|record| match &record.fact {
                NodeFact::ArtifactProduced(fact) => Some(&fact.value),
                _ => None,
            })
            .unwrap();
        assert_eq!(artifact, &serde_json::json!({"value": "[REDACTED]"}));
        let comment = records
            .iter()
            .find_map(|record| match &record.fact {
                NodeFact::ApprovalGranted(fact) => fact.comment.as_deref(),
                _ => None,
            })
            .unwrap();
        assert_eq!(comment, "[REDACTED]");
    }
}
