//! Workflow read-only query service.
//!
//! Query services assemble read models from repository ports only. They do not
//! call command usecases and they do not mutate workflow state.

use std::sync::Arc;

use serde_json::Value;

use crate::domain::workflow::{
    ExecutionTree, ExecutionTreeId, WorkflowDefinition, WorkflowDefinitionName,
    WorkflowDefinitionRepository, WorkflowError,
};

use super::event_draft;
use super::ports::{
    WorkflowEventDraft, WorkflowEventRepository, WorkflowExecutionProjectionRepository,
};

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
    events: Arc<dyn WorkflowEventRepository>,
    execution_projection: Arc<dyn WorkflowExecutionProjectionRepository>,
}

impl WorkflowQueryService {
    pub fn new(
        definitions: Arc<dyn WorkflowDefinitionRepository>,
        events: Arc<dyn WorkflowEventRepository>,
        execution_projection: Arc<dyn WorkflowExecutionProjectionRepository>,
    ) -> Self {
        Self {
            definitions,
            events,
            execution_projection,
        }
    }

    /// 定義を取得し、存在しない場合は空を返す。
    pub fn get_workflow(
        &self,
        file_stem: &str,
    ) -> Result<Option<WorkflowDefinition>, WorkflowError> {
        let name = WorkflowDefinitionName::new(file_stem.to_string())?;
        self.definitions.find(Some(name.as_str()))
    }

    pub(in crate::usecase::workflow) async fn read_events(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        let execution_id = ExecutionTreeId::new(execution_id.to_string())?;
        self.events.read(&execution_id).await
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
