use std::sync::Arc;

use serde_json::Value;

use crate::domain::workflow::{
    contract, secret_masker, ContractValidationResult, FacetKind, FacetRepository,
    SecretSourceGateway, WorkflowError,
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
    facets: Arc<dyn FacetRepository>,
    secrets: Arc<dyn SecretSourceGateway>,
}

impl WorkflowOutputUsecase {
    pub fn new(
        query: WorkflowQueryService,
        facets: Arc<dyn FacetRepository>,
        secrets: Arc<dyn SecretSourceGateway>,
    ) -> Self {
        Self {
            query,
            facets,
            secrets,
        }
    }

    pub fn validate_output(
        &self,
        run_id: &str,
        step_name: &str,
        structured_output: Value,
    ) -> Result<WorkflowValidateOutputResult, WorkflowError> {
        let events = self.query.read_events(run_id)?;
        let contract_type = resolve_step_output_contract_from_drafts(&events, step_name, run_id)?;
        let contract_definition = self.facets.get(FacetKind::Contract, &contract_type).ok();
        let secrets = self.secrets.configured_secret_values()?;
        let redacted = secret_masker::mask_sensitive_structured_output(
            &contract_type,
            structured_output,
            &secrets,
        );
        Ok(
            match contract::validate_contract_value_with_definition(
                redacted,
                contract_definition.as_deref(),
            ) {
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
        run_id: &str,
        step_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        self.query.get_output(run_id, step_name)
    }
}

fn resolve_step_output_contract_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
    run_id: &str,
) -> Result<String, WorkflowError> {
    event_draft::resolve_step_output_contract_from_drafts(events, step_name, run_id).map_err(
        |err| match err {
            contract::ContractLookupError::RunNotFound { run_id } => {
                WorkflowError::external(format!("Workflow run not found: {run_id}"))
            }
            contract::ContractLookupError::InvalidRunStartedPayload { details } => {
                WorkflowError::validation(details)
            }
            contract::ContractLookupError::NoOutputContract {
                workflow_name,
                step,
            } => WorkflowError::external(format!(
                "step '{step}' has no output_contract in workflow '{workflow_name}'"
            )),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        FacetSummary, NodeDefinition, NodeType, RunId, RunListFilter, WorkflowDefinition,
        WorkflowDefinitionRepository, WorkflowRunRecord, WorkflowRunRepository, WorkflowRunSummary,
        WorkflowStateSnapshot, WorkflowSummary,
    };
    use crate::usecase::workflow::ports::{
        WorkflowEventRepository, WorkflowStateProjectionRepository,
        WorkflowStepDetailProjectionRepository,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct NoopRunRepository;

    impl WorkflowRunRepository for NoopRunRepository {
        fn register_active(&self, _run: WorkflowRunRecord) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn complete_run(
            &self,
            _run_id: &RunId,
            _completed: WorkflowRunRecord,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_runs(
            &self,
            _filter: RunListFilter,
        ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
            Ok(Vec::new())
        }

        fn get_run(&self, _run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
            Ok(None)
        }

        fn resolve_active_run_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<RunId>, WorkflowError> {
            Ok(None)
        }

        fn resolve_worktree_by_run(
            &self,
            _run_id: &RunId,
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

        fn read(&self, _run_id: &RunId) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
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

    struct NoopStateProjectionRepository;

    impl WorkflowStateProjectionRepository for NoopStateProjectionRepository {
        fn get_state(
            &self,
            _run_id: &RunId,
        ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
            Ok(None)
        }
    }

    struct NoopStepDetailProjectionRepository;

    impl WorkflowStepDetailProjectionRepository for NoopStepDetailProjectionRepository {
        fn get_step_detail(
            &self,
            _run_id: &RunId,
            _node_name: &str,
            _run_index: Option<u32>,
        ) -> Result<Option<serde_json::Value>, WorkflowError> {
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
        facets: Arc<FakeFacetRepository>,
        events: Arc<FakeEventRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let query = WorkflowQueryService::new(
                Arc::new(NoopRunRepository),
                Arc::new(NoopDefinitionRepository),
                facets.clone(),
                events.clone(),
                Arc::new(NoopStateProjectionRepository),
                Arc::new(NoopStepDetailProjectionRepository),
            );
            let usecase = WorkflowOutputUsecase::new(
                query,
                facets.clone(),
                Arc::new(FakeSecretSourceGateway),
            );
            Self {
                usecase,
                facets,
                events,
            }
        }
    }

    fn definition_with_output_contract(contract: &str) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Agent,
                output_contract: Some(contract.to_string()),
                ..Default::default()
            }],
        }
    }

    fn run_started(run_id: &str, definition: WorkflowDefinition) -> WorkflowEventDraft {
        let workflow_name = definition.name.clone();
        let definition = crate::usecase::workflow::dto::workflow_to_dto(&definition);
        WorkflowEventDraft {
            run_id: run_id.to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_name": workflow_name,
                "workflow_file_stem": "wf",
                "worktree_path": "/wt",
                "workflow_definition": definition,
            }),
        }
    }

    fn output_submitted(
        run_id: &str,
        node_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        timestamp: f64,
        request_id: &str,
    ) -> WorkflowEventDraft {
        WorkflowEventDraft {
            run_id: run_id.to_string(),
            event_kind: "output_submitted".to_string(),
            timestamp,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "node_name": node_name,
                "contract": contract,
                "structured_output": structured_output,
                "submitted_at": timestamp,
                "request_id": request_id,
            }),
        }
    }

    fn test_run_id() -> &'static str {
        "00000000-0000-4000-8000-000000000301"
    }

    #[test]
    fn validate_output_resolves_contract_from_run_started_and_masks_before_validation() {
        let fixture = Fixture::new();
        fixture.events.seed(run_started(
            test_run_id(),
            definition_with_output_contract("review-result"),
        ));
        fixture
            .facets
            .save(
                FacetKind::Contract,
                "review-result",
                r#"```contract-validation
{"required":["status"]}
```"#,
                true,
            )
            .unwrap();

        let result = fixture
            .usecase
            .validate_output(
                test_run_id(),
                "review",
                serde_json::json!({"status":"ok","secret":"token-123"}),
            )
            .unwrap();

        assert_eq!(result, WorkflowValidateOutputResult::Valid);
        let invalid = fixture
            .usecase
            .validate_output(test_run_id(), "review", serde_json::json!({}))
            .unwrap();
        assert!(matches!(
            invalid,
            WorkflowValidateOutputResult::Invalid { reason, .. } if reason == "missing_field"
        ));
    }

    #[test]
    fn get_output_delegates_to_query_projection() {
        let fixture = Fixture::new();
        fixture.events.seed(output_submitted(
            test_run_id(),
            "review",
            "review-result",
            serde_json::json!({"status":"ok"}),
            2.0,
            "req-1",
        ));

        let output = fixture.usecase.get_output(test_run_id(), "review").unwrap();

        assert!(matches!(
            output,
            WorkflowGetOutputResult::Submitted { request_id, .. }
                if request_id.as_deref() == Some("req-1")
        ));
    }
}
