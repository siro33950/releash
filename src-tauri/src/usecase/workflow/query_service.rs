//! Workflow read-only query service.
//!
//! Query services assemble read models from repository ports only. They do not
//! call command usecases and they do not mutate workflow state.

#[cfg(any(test, feature = "test-support"))]
use crate::domain::workflow::WorkflowPageRequest;
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use serde_json::Map;
use serde_json::Value;

use crate::domain::workflow::{
    ExecutionTree, ExecutionTreeId, FacetKind, FacetRepository, FacetSummary, WorkflowDefinition,
    WorkflowDefinitionName, WorkflowDefinitionRepository, WorkflowError, WorkflowSummary,
};

use super::event_draft;
use super::ports::{
    WorkflowDefinitionSourceGateway, WorkflowEventDraft, WorkflowEventRepository,
    WorkflowExecutionProjectionRepository,
};

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEventView {
    pub event: String,
    pub execution_id: String,
    pub timestamp_ms: f64,
    pub payload: Map<String, Value>,
}

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

    pub fn get_workflow_source_format(
        &self,
        file_stem: &str,
    ) -> Result<crate::domain::workflow::WorkflowSourceFormat, WorkflowError> {
        let name = WorkflowDefinitionName::new(file_stem.to_string())?;
        self.definition_sources.source_format(name.as_str())
    }

    pub(in crate::usecase::workflow) async fn read_events(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        let execution_id = ExecutionTreeId::new(execution_id.to_string())?;
        self.events.read(&execution_id).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn get_execution_log_page(
        &self,
        execution_id: &str,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventView>, WorkflowError> {
        let execution_id = ExecutionTreeId::new(execution_id.to_string())?;
        Ok(self
            .events
            .read_page(&execution_id, page)
            .await?
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

    pub(super) fn get_node_output_from_events(
        &self,
        execution_id: &str,
        node_name: &str,
        events: &[WorkflowEventDraft],
    ) -> Result<WorkflowGetOutputResult, WorkflowError> {
        let execution_id = ExecutionTreeId::new(execution_id.to_string())?;
        Ok(
            match self.execution_projection.get_node_artifact_from_events(
                &execution_id,
                node_name,
                events,
            )? {
                Some(artifact) => WorkflowGetOutputResult::Submitted {
                    contract: artifact.contract,
                    structured_output: artifact.value,
                    submitted_at: None,
                    request_id: None,
                    timestamp: artifact.produced_at,
                },
                None => WorkflowGetOutputResult::NotSubmitted,
            },
        )
    }

    pub async fn get_execution_state(
        &self,
        execution_id: &str,
    ) -> Result<Option<ExecutionTree>, WorkflowError> {
        let execution_id = ExecutionTreeId::new(execution_id.to_string())?;
        self.execution_projection.get_execution(&execution_id).await
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

#[cfg(any(test, feature = "test-support"))]
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
    for key in ["event", "execution_id", "timestampMs"] {
        object.remove(key);
    }
    WorkflowEventView {
        event: event.event_kind,
        execution_id: event.execution_id,
        timestamp_ms: seconds_to_ms(event.timestamp),
        payload: object,
    }
}

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
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
#[path = "query_service_test.rs"]
pub(crate) mod query_service_tests;
