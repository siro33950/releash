//! Workflow read-only query service.
//!
//! Query services assemble read models from repository ports only. They do not
//! call command usecases and they do not mutate workflow state.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::domain::workflow::{
    FacetKind, FacetRepository, FacetSummary, NodeExecution, WorkflowDefinition,
    WorkflowDefinitionName, WorkflowDefinitionRepository, WorkflowError, WorkflowExecution,
    WorkflowExecutionId, WorkflowPageRequest, WorkflowSummary,
};

use super::event_draft;
use super::ports::{
    WorkflowDefinitionSourceGateway, WorkflowEventDraft, WorkflowEventRepository,
    WorkflowExecutionProjectionRepository,
};

pub type WorkflowEventView = Value;

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowGetOutputResult {
    Submitted {
        contract: Option<String>,
        structured_output: Value,
        submitted_at: Option<f64>,
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

#[derive(Clone)]
pub struct WorkflowQueryService {
    definitions: Arc<dyn WorkflowDefinitionRepository>,
    definition_sources: Arc<dyn WorkflowDefinitionSourceGateway>,
    facets: Arc<dyn FacetRepository>,
    events: Arc<dyn WorkflowEventRepository>,
    execution_projection: Arc<dyn WorkflowExecutionProjectionRepository>,
}

impl WorkflowQueryService {
    pub fn new(
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        definition_sources: Arc<dyn WorkflowDefinitionSourceGateway>,
        facets: Arc<dyn FacetRepository>,
        events: Arc<dyn WorkflowEventRepository>,
        execution_projection: Arc<dyn WorkflowExecutionProjectionRepository>,
    ) -> Self {
        Self {
            definitions,
            definition_sources,
            facets,
            events,
            execution_projection,
        }
    }

    pub fn list_workflows(
        &self,
        running_names: &[String],
    ) -> Result<Vec<WorkflowSummary>, WorkflowError> {
        self.definitions.list(running_names)
    }

    pub fn get_workflow(
        &self,
        file_stem: &str,
    ) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        let name = WorkflowDefinitionName::new(file_stem.to_string())?;
        self.definitions.get(name.as_str())
    }

    pub fn get_workflow_source(&self, file_stem: &str) -> Result<Option<String>, WorkflowError> {
        let name = WorkflowDefinitionName::new(file_stem.to_string())?;
        self.definition_sources.get_source(name.as_str())
    }

    pub(in crate::usecase::workflow) fn read_events(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        self.events.read(&execution_id)
    }

    pub fn get_execution_log(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        Ok(self
            .read_events(execution_id)?
            .into_iter()
            .map(event_draft_to_log_view)
            .collect())
    }

    pub fn get_execution_log_page(
        &self,
        execution_id: &str,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        Ok(self
            .events
            .read_page(&execution_id, page)?
            .into_iter()
            .map(event_draft_to_log_view)
            .collect())
    }

    pub(in crate::usecase::workflow) fn get_output_from_events(
        events: &[WorkflowEventDraft],
        node_name: &str,
    ) -> WorkflowGetOutputResult {
        latest_artifact_produced_from_drafts(events, node_name)
            .unwrap_or(WorkflowGetOutputResult::NotSubmitted)
    }

    pub fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        self.execution_projection.get_execution(&execution_id)
    }

    pub fn get_node_detail(
        &self,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<NodeExecution>, WorkflowError> {
        Ok(self
            .get_execution_state(execution_id)?
            .and_then(|execution| {
                execution
                    .node_executions
                    .into_iter()
                    .find(|node_execution| node_execution.id == node_execution_id)
            }))
    }

    pub fn list_facets(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
        self.facets.list(kind)
    }

    pub fn get_facet(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
        self.facets.get(kind, key)
    }

    pub fn list_facet_summaries(
        &self,
        kind: FacetKind,
    ) -> Result<Vec<FacetSummary>, WorkflowError> {
        self.facets.list_summaries(kind)
    }
}

fn event_draft_to_log_view(event: WorkflowEventDraft) -> WorkflowEventView {
    let mut object = match event.payload {
        Value::Object(object) => object,
        other => {
            let mut object = Map::new();
            object.insert("payload".to_string(), other);
            object
        }
    };

    rename_seconds_field_to_ms(&mut object, "requested_at", "requestedAtMs");
    rename_seconds_field_to_ms(&mut object, "submitted_at", "submittedAtMs");
    object.insert("event".to_string(), Value::String(event.event_kind));
    object.insert(
        "execution_id".to_string(),
        Value::String(event.execution_id),
    );
    object.insert(
        "timestampMs".to_string(),
        serde_json::json!(seconds_to_ms(event.timestamp)),
    );
    Value::Object(object)
}

fn rename_seconds_field_to_ms(object: &mut Map<String, Value>, source: &str, target: &str) {
    let Some(value) = object.remove(source) else {
        return;
    };
    if let Some(seconds) = value.as_f64() {
        object.insert(
            target.to_string(),
            serde_json::json!(seconds_to_ms(seconds)),
        );
    }
}

fn seconds_to_ms(seconds: f64) -> f64 {
    seconds * 1000.0
}

fn latest_artifact_produced_from_drafts(
    events: &[WorkflowEventDraft],
    node_name: &str,
) -> Option<WorkflowGetOutputResult> {
    event_draft::latest_artifact_produced_from_drafts(events, node_name).map(|snapshot| {
        WorkflowGetOutputResult::Submitted {
            contract: snapshot.contract,
            structured_output: snapshot.value,
            submitted_at: snapshot.submitted_at,
            request_id: snapshot.request_id,
            timestamp: snapshot.timestamp,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, FacetRefs, NodeDefinition, NodeExecution,
        NodeExecutionStatus, NodeKind, NodeKindName, SessionGate, SessionSpec, TokenUsage,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FakeDefinitionRepository {
        workflow: WorkflowDefinition,
    }

    impl crate::domain::workflow::WorkflowDefinitionRepository for FakeDefinitionRepository {
        fn list(&self, running_names: &[String]) -> Result<Vec<WorkflowSummary>, WorkflowError> {
            Ok(vec![WorkflowSummary {
                name: self.workflow.name.clone(),
                description: self.workflow.description.clone(),
                builtin: self.workflow.builtin,
                is_running: running_names.contains(&self.workflow.name),
            }])
        }

        fn get(&self, file_stem: &str) -> Result<Option<WorkflowDefinition>, WorkflowError> {
            Ok((file_stem == self.workflow.name).then(|| self.workflow.clone()))
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

    struct FakeDefinitionSourceGateway {
        workflow_source: Option<String>,
    }

    impl WorkflowDefinitionSourceGateway for FakeDefinitionSourceGateway {
        fn get_source(&self, _file_stem: &str) -> Result<Option<String>, WorkflowError> {
            Ok(self.workflow_source.clone())
        }

        fn save_source(
            &self,
            _source: &str,
            _original_name: Option<&str>,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            Err(WorkflowError::external("not used"))
        }
    }

    #[derive(Default)]
    struct FakeFacetRepository {
        values: Mutex<HashMap<(FacetKind, String), String>>,
    }

    impl crate::domain::workflow::FacetRepository for FakeFacetRepository {
        fn list(&self, kind: FacetKind) -> Result<Vec<String>, WorkflowError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .keys()
                .filter(|(candidate, _)| *candidate == kind)
                .map(|(_, key)| key.clone())
                .collect())
        }

        fn get(&self, kind: FacetKind, key: &str) -> Result<String, WorkflowError> {
            self.values
                .lock()
                .unwrap()
                .get(&(kind, key.to_string()))
                .cloned()
                .ok_or_else(|| WorkflowError::NotFound(key.to_string()))
        }

        fn save(
            &self,
            _kind: FacetKind,
            _key: &str,
            _content: &str,
            _is_new: bool,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn delete(&self, _kind: FacetKind, _key: &str) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_summaries(&self, kind: FacetKind) -> Result<Vec<FacetSummary>, WorkflowError> {
            Ok(self
                .list(kind)?
                .into_iter()
                .map(|key| FacetSummary {
                    key,
                    // 本番 (gateway facet.rs) と同じ canonical 語彙をテストでも再現する。
                    kind: match kind {
                        FacetKind::Policy => "policy",
                        FacetKind::Knowledge => "knowledge",
                        FacetKind::Instruction => "instruction",
                    }
                    .to_string(),
                    description: String::new(),
                    builtin: false,
                })
                .collect())
        }
    }

    #[derive(Default)]
    struct FakeEventRepository {
        events: Mutex<Vec<WorkflowEventDraft>>,
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
    struct FakeExecutionProjectionRepository {
        executions: Mutex<HashMap<String, WorkflowExecution>>,
    }

    impl FakeExecutionProjectionRepository {
        fn seed(&self, execution: WorkflowExecution) {
            self.executions
                .lock()
                .unwrap()
                .insert(execution.id.clone(), execution);
        }
    }

    impl WorkflowExecutionProjectionRepository for FakeExecutionProjectionRepository {
        fn get_execution(
            &self,
            execution_id: &WorkflowExecutionId,
        ) -> Result<Option<WorkflowExecution>, WorkflowError> {
            Ok(self
                .executions
                .lock()
                .unwrap()
                .get(execution_id.as_str())
                .cloned())
        }
    }

    struct Fixture {
        service: WorkflowQueryService,
        facets: Arc<FakeFacetRepository>,
        events: Arc<FakeEventRepository>,
        projections: Arc<FakeExecutionProjectionRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let definitions = Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
            });
            let definition_sources = Arc::new(FakeDefinitionSourceGateway {
                workflow_source: None,
            });
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let projections = Arc::new(FakeExecutionProjectionRepository::default());
            let service = WorkflowQueryService::new(
                definitions,
                definition_sources,
                facets.clone(),
                events.clone(),
                projections.clone(),
            );
            Self {
                service,
                facets,
                events,
                projections,
            }
        }
    }

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    gate: SessionGate::Approval,
                    facets: FacetRefs {
                        instruction: Some("implement".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
        }
    }

    fn execution_projection(execution_id: &str) -> WorkflowExecution {
        WorkflowExecution {
            id: execution_id.to_string(),
            workflow_name: "wf".to_string(),
            status: ExecutionStatus::Running,
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
            node_executions: vec![NodeExecution {
                id: "ne-review-1".to_string(),
                execution_id: execution_id.to_string(),
                node_name: "review".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: None,
                display_command: None,
                result_summary: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                completion_signals: Default::default(),
                started_at: 1.0,
                completed_at: None,
            }],
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    fn test_execution_id() -> &'static str {
        "00000000-0000-4000-8000-000000000101"
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

    #[test]
    fn workflow_queries_delegate_to_definition_repository() {
        let fixture = Fixture::new();
        let summaries = fixture.service.list_workflows(&["wf".to_string()]).unwrap();
        assert_eq!(summaries[0].name, "wf");
        assert!(summaries[0].is_running);
        assert!(fixture.service.get_workflow("wf").unwrap().is_some());
        assert!(fixture.service.get_workflow("missing").unwrap().is_none());
        assert!(fixture.service.get_workflow("bad name!").is_err());
        assert!(fixture.service.get_workflow_source("bad name!").is_err());
    }

    #[test]
    fn event_and_facet_queries_validate_execution_ids_and_delegate() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "execution_started".to_string(),
                timestamp: 1.0,
                payload: serde_json::json!({}),
            })
            .unwrap();
        fixture.facets.values.lock().unwrap().insert(
            (FacetKind::Instruction, "implement".to_string()),
            "instruction body".to_string(),
        );

        assert_eq!(
            fixture
                .service
                .read_events(test_execution_id())
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            fixture.service.read_events("not-a-uuid").unwrap_err(),
            WorkflowError::Validation(_)
        ));
        assert_eq!(
            fixture
                .service
                .get_facet(FacetKind::Instruction, "implement")
                .unwrap(),
            "instruction body"
        );
        assert_eq!(
            fixture
                .service
                .list_facet_summaries(FacetKind::Instruction)
                .unwrap()[0]
                .key,
            "implement"
        );
    }

    #[test]
    fn get_execution_log_projects_event_drafts_to_wire_timestamp_fields() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "execution_started".to_string(),
                timestamp: 1.25,
                payload: serde_json::json!({
                    "workflow_name": "wf",
                    "worktree_path": "/wt",
                }),
            })
            .unwrap();

        let events = fixture
            .service
            .get_execution_log(test_execution_id())
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "execution_started");
        assert_eq!(events[0]["execution_id"], test_execution_id());
        assert_eq!(events[0]["workflow_name"], "wf");
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(1250.0));
        assert!(events[0].get("timestamp").is_none());
    }

    #[test]
    fn get_execution_log_page_projects_only_the_requested_event_window() {
        let fixture = Fixture::new();
        for (event_kind, timestamp) in [("execution_started", 1.0), ("node_started", 2.0)] {
            fixture
                .events
                .append(&WorkflowEventDraft {
                    execution_id: test_execution_id().to_string(),
                    event_kind: event_kind.to_string(),
                    timestamp,
                    payload: serde_json::json!({}),
                })
                .unwrap();
        }

        let events = fixture
            .service
            .get_execution_log_page(test_execution_id(), WorkflowPageRequest::new(1, 1))
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "node_started");
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(2000.0));
    }

    #[test]
    fn get_execution_log_renames_submission_timestamp_to_millisecond_field() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "artifact_produced".to_string(),
                timestamp: 4.0,
                payload: serde_json::json!({
                    "node_execution_id": format!("{}:review:1", test_execution_id()),
                    "node_name": "review",
                    "contract": "review-result",
                    "value": {"status": "ok"},
                    "submitted_at": 4.0,
                    "request_id": "req-2",
                }),
            })
            .unwrap();

        let events = fixture
            .service
            .get_execution_log(test_execution_id())
            .unwrap();

        assert_eq!(events[0]["submittedAtMs"].as_f64(), Some(4000.0));
        assert!(events[0].get("submitted_at").is_none());
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(4000.0));
    }

    #[test]
    fn get_output_returns_latest_submitted_snapshot_for_node() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&artifact_produced(
                test_execution_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"old"}),
                2.0,
                "req-old",
            ))
            .unwrap();
        fixture
            .events
            .append(&artifact_produced(
                test_execution_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"new"}),
                3.0,
                "req-new",
            ))
            .unwrap();

        let events = fixture.service.read_events(test_execution_id()).unwrap();
        let result = WorkflowQueryService::get_output_from_events(&events, "review");

        assert_eq!(
            result,
            WorkflowGetOutputResult::Submitted {
                contract: Some("review-result".to_string()),
                structured_output: serde_json::json!({"status":"new"}),
                submitted_at: Some(3.0),
                request_id: Some("req-new".to_string()),
                timestamp: 3.0,
            }
        );
        assert_eq!(
            WorkflowQueryService::get_output_from_events(&events, "missing"),
            WorkflowGetOutputResult::NotSubmitted
        );
    }

    #[test]
    fn get_output_returns_contractless_standard_artifact_for_node() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                execution_id: test_execution_id().to_string(),
                event_kind: "artifact_produced".to_string(),
                timestamp: 4.0,
                payload: serde_json::json!({
                    "node_execution_id": format!("{}:review:1", test_execution_id()),
                    "node_name": "review",
                    "contract": null,
                    "value": {
                        "ok": false,
                        "exit_code": 7,
                        "stdout": "out",
                        "stderr": "err",
                        "duration": 10
                    }
                }),
            })
            .unwrap();

        let events = fixture.service.read_events(test_execution_id()).unwrap();
        let result = WorkflowQueryService::get_output_from_events(&events, "review");

        assert_eq!(
            result,
            WorkflowGetOutputResult::Submitted {
                contract: None,
                structured_output: serde_json::json!({
                    "ok": false,
                    "exit_code": 7,
                    "stdout": "out",
                    "stderr": "err",
                    "duration": 10
                }),
                submitted_at: None,
                request_id: None,
                timestamp: 4.0,
            }
        );
    }

    #[test]
    fn get_execution_state_delegates_to_execution_projection_port() {
        let fixture = Fixture::new();
        fixture
            .projections
            .seed(execution_projection(test_execution_id()));

        let state = fixture
            .service
            .get_execution_state(test_execution_id())
            .unwrap()
            .unwrap();

        assert_eq!(state.id, test_execution_id());
        assert_eq!(state.workflow_name, "wf");
        assert_eq!(state.node_executions[0].id, "ne-review-1");
        assert_eq!(state.node_executions[0].node_name, "review");
        assert!(fixture.service.get_execution_state("not-a-uuid").is_err());
    }

    #[test]
    fn get_node_detail_is_derived_from_the_execution_projection() {
        let fixture = Fixture::new();
        fixture
            .projections
            .seed(execution_projection(test_execution_id()));

        let detail = fixture
            .service
            .get_node_detail(test_execution_id(), "ne-review-1")
            .unwrap()
            .unwrap();

        assert_eq!(detail.node_name, "review");
        assert_eq!(detail.attempt, 1);
        assert!(fixture
            .service
            .get_node_detail("not-a-uuid", "ne-review-1")
            .is_err());
    }
}
