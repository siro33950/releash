//! Workflow read-only query service.
//!
//! Query services assemble read models from repository ports only. They do not
//! call command usecases and they do not mutate workflow state.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::domain::workflow::{
    FacetKind, FacetRepository, FacetSummary, RunId, RunListFilter, WorkflowDefinition,
    WorkflowDefinitionRepository, WorkflowError, WorkflowName, WorkflowRunRepository,
    WorkflowRunSummary, WorkflowStateSnapshot, WorkflowSummary,
};

use super::event_draft;
use super::ports::{
    WorkflowEventDraft, WorkflowEventRepository, WorkflowStateProjectionRepository,
    WorkflowStepDetailProjectionRepository,
};

pub type WorkflowEventView = Value;
pub type WorkflowStepDetailView = Value;

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowGetOutputResult {
    Submitted {
        contract: String,
        structured_output: Value,
        submitted_at: Option<f64>,
        request_id: Option<String>,
        timestamp: f64,
    },
    NotSubmitted,
}

#[derive(Clone)]
pub struct WorkflowQueryService {
    runs: Arc<dyn WorkflowRunRepository>,
    definitions: Arc<dyn WorkflowDefinitionRepository>,
    facets: Arc<dyn FacetRepository>,
    events: Arc<dyn WorkflowEventRepository>,
    state_projection: Arc<dyn WorkflowStateProjectionRepository>,
    step_details: Arc<dyn WorkflowStepDetailProjectionRepository>,
}

impl WorkflowQueryService {
    pub fn new(
        runs: Arc<dyn WorkflowRunRepository>,
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        facets: Arc<dyn FacetRepository>,
        events: Arc<dyn WorkflowEventRepository>,
        state_projection: Arc<dyn WorkflowStateProjectionRepository>,
        step_details: Arc<dyn WorkflowStepDetailProjectionRepository>,
    ) -> Self {
        Self {
            runs,
            definitions,
            facets,
            events,
            state_projection,
            step_details,
        }
    }

    pub fn list_runs(
        &self,
        filter: RunListFilter,
    ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
        self.runs.list_runs(filter)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        self.runs.get_run(&run_id)
    }

    pub fn resolve_worktree_by_run(&self, run_id: &str) -> Result<Option<String>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        self.runs.resolve_worktree_by_run(&run_id)
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
        let name = WorkflowName::new(file_stem.to_string())?;
        self.definitions.get(name.as_str())
    }

    pub(in crate::usecase::workflow) fn read_events(
        &self,
        run_id: &str,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        self.events.read(&run_id)
    }

    pub fn get_run_log(&self, run_id: &str) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        Ok(self
            .read_events(run_id)?
            .into_iter()
            .map(event_draft_to_log_view)
            .collect())
    }

    pub fn get_output(
        &self,
        run_id: &str,
        step_name: &str,
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        let events = self.read_events(run_id)?;
        Ok(latest_output_submitted_from_drafts(&events, step_name)
            .unwrap_or(WorkflowGetOutputResult::NotSubmitted))
    }

    pub fn get_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        self.state_projection.get_state(&run_id)
    }

    pub fn get_step_detail(
        &self,
        run_id: &str,
        node_name: &str,
        run_index: Option<u32>,
    ) -> Result<Option<WorkflowStepDetailView>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        self.step_details
            .get_step_detail(&run_id, node_name, run_index)
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
    object.insert("run_id".to_string(), Value::String(event.run_id));
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

fn latest_output_submitted_from_drafts(
    events: &[WorkflowEventDraft],
    step_name: &str,
) -> Option<WorkflowGetOutputResult> {
    event_draft::latest_output_submitted_from_drafts(events, step_name).map(|snapshot| {
        WorkflowGetOutputResult::Submitted {
            contract: snapshot.contract,
            structured_output: snapshot.structured_output,
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
        NodeDefinition, NodeType, RunStatus, RunStatusFilter, TriggerSource,
        WorkflowExecutionState, WorkflowRunRecord,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunRepository {
        runs: Mutex<HashMap<String, WorkflowRunSummary>>,
    }

    impl FakeRunRepository {
        fn seed(&self, run: WorkflowRunSummary) {
            self.runs.lock().unwrap().insert(run.run_id.clone(), run);
        }
    }

    impl crate::domain::workflow::WorkflowRunRepository for FakeRunRepository {
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

        fn cancel_reservation(&self, _run_id: &RunId) -> Result<(), WorkflowError> {
            Ok(())
        }

        fn list_runs(
            &self,
            filter: RunListFilter,
        ) -> Result<Vec<WorkflowRunSummary>, WorkflowError> {
            let mut runs: Vec<_> = self.runs.lock().unwrap().values().cloned().collect();
            if let Some(status) = filter.status {
                runs.retain(|run| match status {
                    RunStatusFilter::Active => !run.status.is_terminal(),
                    RunStatusFilter::Terminal => run.status.is_terminal(),
                    RunStatusFilter::All => true,
                });
            }
            if let Some(worktree_path) = filter.worktree_path {
                runs.retain(|run| run.worktree_path == worktree_path);
            }
            Ok(runs)
        }

        fn get_run(&self, run_id: &RunId) -> Result<Option<WorkflowRunSummary>, WorkflowError> {
            Ok(self.runs.lock().unwrap().get(run_id.as_str()).cloned())
        }

        fn resolve_active_run_by_worktree(
            &self,
            worktree_path: &str,
        ) -> Result<Option<RunId>, WorkflowError> {
            self.runs
                .lock()
                .unwrap()
                .values()
                .find(|run| run.worktree_path == worktree_path && !run.status.is_terminal())
                .map(|run| RunId::new(run.run_id.clone()))
                .transpose()
        }

        fn resolve_worktree_by_run(&self, run_id: &RunId) -> Result<Option<String>, WorkflowError> {
            Ok(self
                .runs
                .lock()
                .unwrap()
                .get(run_id.as_str())
                .map(|run| run.worktree_path.clone()))
        }
    }

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
                    kind: kind.dir_name().to_string(),
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

        fn read(&self, _run_id: &RunId) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    #[derive(Default)]
    struct FakeStateProjectionRepository {
        states: Mutex<HashMap<String, WorkflowStateSnapshot>>,
    }

    impl FakeStateProjectionRepository {
        fn seed(&self, state: WorkflowStateSnapshot) {
            self.states
                .lock()
                .unwrap()
                .insert(state.execution_id.clone(), state);
        }
    }

    impl WorkflowStateProjectionRepository for FakeStateProjectionRepository {
        fn get_state(
            &self,
            run_id: &RunId,
        ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
            Ok(self.states.lock().unwrap().get(run_id.as_str()).cloned())
        }
    }

    type StepDetailKey = (String, String, Option<u32>);

    #[derive(Default)]
    struct FakeStepDetailProjectionRepository {
        details: Mutex<HashMap<StepDetailKey, serde_json::Value>>,
    }

    impl FakeStepDetailProjectionRepository {
        fn seed(
            &self,
            run_id: &str,
            node_name: &str,
            run_index: Option<u32>,
            detail: serde_json::Value,
        ) {
            self.details.lock().unwrap().insert(
                (run_id.to_string(), node_name.to_string(), run_index),
                detail,
            );
        }
    }

    impl WorkflowStepDetailProjectionRepository for FakeStepDetailProjectionRepository {
        fn get_step_detail(
            &self,
            run_id: &RunId,
            node_name: &str,
            run_index: Option<u32>,
        ) -> Result<Option<serde_json::Value>, WorkflowError> {
            Ok(self
                .details
                .lock()
                .unwrap()
                .get(&(
                    run_id.as_str().to_string(),
                    node_name.to_string(),
                    run_index,
                ))
                .cloned())
        }
    }

    struct Fixture {
        service: WorkflowQueryService,
        runs: Arc<FakeRunRepository>,
        facets: Arc<FakeFacetRepository>,
        events: Arc<FakeEventRepository>,
        states: Arc<FakeStateProjectionRepository>,
        step_details: Arc<FakeStepDetailProjectionRepository>,
    }

    impl Fixture {
        fn new() -> Self {
            let runs = Arc::new(FakeRunRepository::default());
            let definitions = Arc::new(FakeDefinitionRepository {
                workflow: workflow(),
            });
            let facets = Arc::new(FakeFacetRepository::default());
            let events = Arc::new(FakeEventRepository::default());
            let states = Arc::new(FakeStateProjectionRepository::default());
            let step_details = Arc::new(FakeStepDetailProjectionRepository::default());
            let service = WorkflowQueryService::new(
                runs.clone(),
                definitions,
                facets.clone(),
                events.clone(),
                states.clone(),
                step_details.clone(),
            );
            Self {
                service,
                runs,
                facets,
                events,
                states,
                step_details,
            }
        }
    }

    fn workflow() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: "desc".to_string(),
            builtin: false,
            variables: Default::default(),
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Approval,
                ..Default::default()
            }],
        }
    }

    fn run(run_id: &str, status: RunStatus, worktree_path: &str) -> WorkflowRunSummary {
        WorkflowRunSummary {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: None,
            status,
            worktree_path: worktree_path.to_string(),
            current_node_name: Some("review".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 1.0,
            completed_at: None,
            error_reason: None,
        }
    }

    fn state_snapshot(run_id: &str) -> WorkflowStateSnapshot {
        WorkflowStateSnapshot {
            execution_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "review".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: Vec::new(),
            step_execution_counts: HashMap::new(),
            workflow_definition: workflow(),
            total_token_usage: Default::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: Vec::new(),
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 1.0,
        }
    }

    fn test_run_id() -> &'static str {
        "00000000-0000-4000-8000-000000000101"
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

    #[test]
    fn list_runs_applies_repository_filters() {
        let fixture = Fixture::new();
        fixture
            .runs
            .seed(run(test_run_id(), RunStatus::Running, "/repo/a"));
        fixture.runs.seed(run(
            "00000000-0000-4000-8000-000000000102",
            RunStatus::Completed,
            "/repo/a",
        ));
        fixture.runs.seed(run(
            "00000000-0000-4000-8000-000000000103",
            RunStatus::Running,
            "/repo/b",
        ));

        let runs = fixture
            .service
            .list_runs(RunListFilter {
                status: Some(RunStatusFilter::Active),
                worktree_path: Some("/repo/a".to_string()),
            })
            .unwrap();

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, test_run_id());
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
    }

    #[test]
    fn event_and_facet_queries_validate_run_ids_and_delegate() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                run_id: test_run_id().to_string(),
                event_kind: "run_started".to_string(),
                timestamp: 1.0,
                payload: serde_json::json!({}),
            })
            .unwrap();
        fixture.facets.values.lock().unwrap().insert(
            (FacetKind::Contract, "spec-directory".to_string()),
            "contract body".to_string(),
        );

        assert_eq!(fixture.service.read_events(test_run_id()).unwrap().len(), 1);
        assert!(matches!(
            fixture.service.read_events("not-a-uuid").unwrap_err(),
            WorkflowError::Validation(_)
        ));
        assert_eq!(
            fixture
                .service
                .get_facet(FacetKind::Contract, "spec-directory")
                .unwrap(),
            "contract body"
        );
        assert_eq!(
            fixture
                .service
                .list_facet_summaries(FacetKind::Contract)
                .unwrap()[0]
                .key,
            "spec-directory"
        );
    }

    #[test]
    fn get_run_log_projects_event_drafts_to_wire_timestamp_fields() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                run_id: test_run_id().to_string(),
                event_kind: "run_started".to_string(),
                timestamp: 1.25,
                payload: serde_json::json!({
                    "workflow_name": "wf",
                    "workflow_file_stem": "wf",
                    "worktree_path": "/wt",
                }),
            })
            .unwrap();

        let events = fixture.service.get_run_log(test_run_id()).unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "run_started");
        assert_eq!(events[0]["run_id"], test_run_id());
        assert_eq!(events[0]["workflow_name"], "wf");
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(1250.0));
        assert!(events[0].get("timestamp").is_none());
    }

    #[test]
    fn get_run_log_renames_caller_timestamps_to_millisecond_fields() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&WorkflowEventDraft {
                run_id: test_run_id().to_string(),
                event_kind: "cli_mutation_requested".to_string(),
                timestamp: 3.0,
                payload: serde_json::json!({
                    "workflow_name": "wf",
                    "request_id": "req-1",
                    "request": {"type": "abort"},
                    "requested_at": 2.0,
                }),
            })
            .unwrap();
        fixture
            .events
            .append(&WorkflowEventDraft {
                run_id: test_run_id().to_string(),
                event_kind: "output_submitted".to_string(),
                timestamp: 4.0,
                payload: serde_json::json!({
                    "workflow_name": "wf",
                    "node_name": "review",
                    "contract": "review-result",
                    "structured_output": {"status": "ok"},
                    "submitted_at": 4.0,
                    "request_id": "req-2",
                }),
            })
            .unwrap();

        let events = fixture.service.get_run_log(test_run_id()).unwrap();

        assert_eq!(events[0]["requestedAtMs"].as_f64(), Some(2000.0));
        assert!(events[0].get("requested_at").is_none());
        assert_eq!(events[0]["timestampMs"].as_f64(), Some(3000.0));
        assert_eq!(events[1]["submittedAtMs"].as_f64(), Some(4000.0));
        assert!(events[1].get("submitted_at").is_none());
        assert_eq!(events[1]["timestampMs"].as_f64(), Some(4000.0));
    }

    #[test]
    fn get_output_returns_latest_submitted_snapshot_for_step() {
        let fixture = Fixture::new();
        fixture
            .events
            .append(&output_submitted(
                test_run_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"old"}),
                2.0,
                "req-old",
            ))
            .unwrap();
        fixture
            .events
            .append(&output_submitted(
                test_run_id(),
                "review",
                "review-result",
                serde_json::json!({"status":"new"}),
                3.0,
                "req-new",
            ))
            .unwrap();

        let result = fixture.service.get_output(test_run_id(), "review").unwrap();

        assert_eq!(
            result,
            WorkflowGetOutputResult::Submitted {
                contract: "review-result".to_string(),
                structured_output: serde_json::json!({"status":"new"}),
                submitted_at: Some(3.0),
                request_id: Some("req-new".to_string()),
                timestamp: 3.0,
            }
        );
        assert_eq!(
            fixture
                .service
                .get_output(test_run_id(), "missing")
                .unwrap(),
            WorkflowGetOutputResult::NotSubmitted
        );
    }

    #[test]
    fn get_run_state_delegates_to_state_projection_port() {
        let fixture = Fixture::new();
        fixture.states.seed(state_snapshot(test_run_id()));

        let state = fixture
            .service
            .get_run_state(test_run_id())
            .unwrap()
            .unwrap();

        assert_eq!(state.execution_id, test_run_id());
        assert_eq!(state.workflow_name, "wf");
        assert!(fixture.service.get_run_state("not-a-uuid").is_err());
    }

    #[test]
    fn get_step_detail_delegates_to_step_detail_projection_port() {
        let fixture = Fixture::new();
        fixture.step_details.seed(
            test_run_id(),
            "review",
            Some(2),
            serde_json::json!({"stepName":"review","runIndex":2}),
        );

        let detail = fixture
            .service
            .get_step_detail(test_run_id(), "review", Some(2))
            .unwrap()
            .unwrap();

        assert_eq!(detail["stepName"], "review");
        assert_eq!(detail["runIndex"].as_u64(), Some(2));
        assert!(fixture
            .service
            .get_step_detail("not-a-uuid", "review", None)
            .is_err());
    }
}
