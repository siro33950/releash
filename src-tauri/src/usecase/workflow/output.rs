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

    pub fn validate_output(
        &self,
        execution_id: &str,
        node_name: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        let events = self.query.read_events(execution_id)?;
        let context = resolve_node_artifact_schema_from_drafts(&events, node_name, execution_id)?;
        self.validate_with_context(context, structured_output)
    }

    pub fn validate_output_for_contract(
        &self,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        let events = self.query.read_events(execution_id)?;
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

    pub fn get_output(
        &self,
        execution_id: &str,
        node_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        let events = self.query.read_events(execution_id)?;
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
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionListFilter, FacetKind, FacetRefs, FacetRepository, FacetSummary, NodeDefinition,
        NodeKind, SchemaDef, SessionSpec, WorkflowDefinition, WorkflowDefinitionRepository,
        WorkflowExecution, WorkflowExecutionId, WorkflowExecutionRecord,
        WorkflowExecutionRepository, WorkflowExecutionSummary, WorkflowSummary,
    };
    use crate::usecase::workflow::ports::{
        WorkflowEventRepository, WorkflowExecutionProjectionRepository,
    };
    use crate::usecase::workflow::test_support::NoopDefinitionSourceGateway;
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::sync::Mutex;

    #[derive(Default)]
    struct NoopExecutionRepository;

    impl WorkflowExecutionRepository for NoopExecutionRepository {
        fn register_active(
            &self,
            _execution: WorkflowExecutionRecord,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn complete_execution(
            &self,
            _execution_id: &WorkflowExecutionId,
            _completed: WorkflowExecutionRecord,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_executions(
            &self,
            _filter: ExecutionListFilter,
        ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get_execution(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
            Ok(None)
        }

        fn resolve_active_execution_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<WorkflowExecutionId>, WorkflowError> {
            Ok(None)
        }

        fn resolve_worktree_by_execution(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Option<String>, WorkflowError> {
            Ok(None)
        }
    }

    struct NoopDefinitionRepository;

    impl WorkflowDefinitionRepository for NoopDefinitionRepository {
        fn list(&self, _running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get(&self, _file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok(None)
        }

        fn save(
            &self,
            _definition: WorkflowDefinition,
            _original_name: Option<&str>,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn delete(&self, _name: &str) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeEventRepository {
        events: Mutex<Vec<WorkflowEventDraft>>,
    }

    impl FakeEventRepository {
        fn seed(&self, event: WorkflowEventDraft) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl WorkflowEventRepository for FakeEventRepository {
        fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }

        fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError> {
            self.events.lock().unwrap().extend_from_slice(events);
            Ok(())
        }

        fn read(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    #[derive(Default)]
    struct FakeFacetRepository {
        facets: Mutex<HashMap<(FacetKind, String), String>>,
    }

    impl FacetRepository for FakeFacetRepository {
        fn list(&self, _kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
            self.facets
                .lock()
                .unwrap()
                .get(&(kind, key.to_string()))
                .cloned()
                .ok_or_else(|| WorkflowError::NotFound(key.to_string()))
        }

        fn save(
            &self,
            kind: FacetKind,
            key: &str,
            content: &str,
            _is_new: bool,
        ) -> Result<(), WorkflowError> {
            self.facets
                .lock()
                .unwrap()
                .insert((kind, key.to_string()), content.to_string());
            Ok(())
        }

        fn delete(&self, kind: FacetKind, key: &str) -> Result<(), WorkflowError> {
            self.facets.lock().unwrap().remove(&(kind, key.to_string()));
            Ok(())
        }

        fn list_summaries(&self, _kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
            Ok(Vec::new())
        }
    }

    struct NoopExecutionProjectionRepository;

    impl WorkflowExecutionProjectionRepository for NoopExecutionProjectionRepository {
        fn get_execution(
            &self,
            _execution_id: &WorkflowExecutionId,
        ) -> Result<Option<WorkflowExecution>, WorkflowError> {
            Ok(None)
        }
    }

    struct FakeSecretSourceGateway;

    impl SecretSourceGateway for FakeSecretSourceGateway {
        fn configured_secret_values(&self) -> Result<Vec<String>, WorkflowError> {
            Ok(vec!["token-123".to_string()])
        }
    }

    struct Fixture {
        usecase: WorkflowOutputUsecase,
        events: Arc<FakeEventRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let query = WorkflowQueryService::new(
                Arc::new(NoopExecutionRepository),
                Arc::new(NoopDefinitionRepository),
                Arc::new(NoopDefinitionSourceGateway),
                facets.clone(),
                events.clone(),
                Arc::new(NoopExecutionProjectionRepository),
            );
            let usecase = WorkflowOutputUsecase::new(query, Arc::new(FakeSecretSourceGateway));
            Self { usecase, events }
        }
    }

    fn definition_with_artifact_contract(contract: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: BTreeMap::from([(
                contract.to_string(),
                SchemaDef::Object {
                    properties: BTreeMap::from([(
                        "status".to_string(),
                        SchemaDef::String { r#enum: None },
                    )]),
                    required: BTreeSet::from(["status".to_string()]),
                },
            )]),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                artifact: Some(contract.to_string()),
                ..Default::default()
            }],
        }
    }

    fn execution_started(execution_id: &str, definition: WorkflowDefinition) -> WorkflowEventDraft {
        let definition = crate::usecase::workflow::dto::workflow_to_dto(&definition);
        WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "definition": definition,
            }),
        }
    }

    fn artifact_produced(
        execution_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        timestamp: f64,
        request_id: &str,
    ) -> WorkflowEventDraft {
        WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "artifact_produced".to_string(),
            timestamp,
            payload: serde_json::json!({
                "node_execution_id": format!("{execution_id}:{node_name}:1"),
                "node_name": node_name,
                "contract": contract,
                "value": structured_output,
                "submitted_at": timestamp,
                "request_id": request_id,
            }),
        }
    }

    fn test_execution_id() -> &'static str {
        "00000000-0000-4000-8000-000000000301"
    }

    #[test]
    fn validate_output_resolves_contract_from_execution_started_and_masks_before_validation() {
        let fixture = Fixture::new();
        fixture.events.seed(execution_started(
            test_execution_id(),
            definition_with_artifact_contract("review-result"),
        ));
        let result = fixture
            .usecase
            .validate_output(
                test_execution_id(),
                "review",
                serde_json::json!({"status":"ok","secret":"token-123"}),
            )
            .unwrap();

        assert_eq!(result, WorkflowValidateOutputResult::Valid);
        let invalid = fixture
            .usecase
            .validate_output(test_execution_id(), "review", serde_json::json!({}))
            .unwrap();
        assert!(matches!(
            invalid,
            WorkflowValidateOutputResult::Invalid { reason, .. } if reason == "schema_violation"
        ));
    }

    #[test]
    fn validate_output_for_contract_rejects_a_mismatched_contract() {
        let fixture = Fixture::new();
        fixture.events.seed(execution_started(
            test_execution_id(),
            definition_with_artifact_contract("review-result"),
        ));

        let error = fixture
            .usecase
            .validate_output_for_contract(
                test_execution_id(),
                "review",
                "different-result",
                serde_json::json!({"status":"ok"}),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            WorkflowError::Validation(message)
                if message.contains("expects contract 'review-result'")
        ));
    }

    #[test]
    fn get_output_delegates_to_query_projection() {
        let fixture = Fixture::new();
        fixture.events.seed(execution_started(
            test_execution_id(),
            definition_with_artifact_contract("review-result"),
        ));
        fixture.events.seed(artifact_produced(
            test_execution_id(),
            "review",
            "review-result",
            serde_json::json!({"status":"ok"}),
            2.0,
            "req-1",
        ));

        let output = fixture
            .usecase
            .get_output(test_execution_id(), "review")
            .unwrap();

        assert!(matches!(
            output,
            WorkflowGetOutputResult::Submitted { request_id, .. }
                if request_id.as_deref() == Some("req-1")
        ));
    }

    #[test]
    fn get_output_rejects_an_unknown_node_through_the_shared_usecase() {
        let fixture = Fixture::new();
        fixture.events.seed(execution_started(
            test_execution_id(),
            definition_with_artifact_contract("review-result"),
        ));

        let error = fixture
            .usecase
            .get_output(test_execution_id(), "missing")
            .unwrap_err();

        assert!(matches!(
            error,
            WorkflowError::Validation(message) if message.contains("is not defined")
        ));
    }
}
