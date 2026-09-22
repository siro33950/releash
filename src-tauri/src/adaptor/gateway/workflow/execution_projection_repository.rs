use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::workflow::services::fact_replay;
use crate::domain::workflow::{ExecutionTree, ExecutionTreeId, WorkflowError};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowExecutionProjectionRepository};

/// 事実ログ（node_events）の tree fold から実行 read model を導出する。
#[derive(Clone)]
pub(crate) struct WorkflowExecutionProjectionLogRepository {
    backend: FactLogReadBackend,
    pub(crate) processes: Option<Arc<dyn crate::domain::workflow::NodeProcessReader>>,
}

impl WorkflowExecutionProjectionLogRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Self {
        Self {
            backend: FactLogReadBackend::Live(store),
            processes: None,
        }
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            backend: FactLogReadBackend::ReadOnly(store),
            processes: None,
        }
    }
}

impl WorkflowExecutionProjectionRepository for WorkflowExecutionProjectionLogRepository {
    fn get_node_artifact_from_events(
        &self,
        execution_id: &ExecutionTreeId,
        node_name: &str,
        events: &[WorkflowEventDraft],
    ) -> Result<Option<crate::domain::workflow::Artifact>, WorkflowError> {
        let records = records_from_drafts(events)?;
        crate::domain::workflow::services::artifact_query::derive_node_artifact(
            execution_id.as_str(),
            &records,
            node_name,
        )
        .map_err(WorkflowError::external)
    }

    fn get_execution(
        &self,
        execution_id: &ExecutionTreeId,
    ) -> Result<Option<ExecutionTree>, WorkflowError> {
        let records = fact_log::read_tree_records_from(&self.backend, execution_id.as_str())
            .map_err(WorkflowError::external)?;
        let Some(tree) = fact_replay::fold_execution_tree(execution_id.as_str(), &records)
            .map_err(WorkflowError::external)?
        else {
            return Ok(None);
        };
        let mut model = fact_replay::derive_read_model(&tree);
        if let Some(processes) = &self.processes {
            for node in &mut model.node_executions {
                node.process_presence = processes.presence(
                    &model.worktree_path,
                    &node.id,
                    node.kind,
                    node.session_id.as_deref(),
                )?;
            }
        }
        Ok(Some(model))
    }
}

fn records_from_drafts(
    events: &[WorkflowEventDraft],
) -> Result<Vec<crate::domain::workflow::NodeFactRecord>, WorkflowError> {
    use crate::adaptor::gateway::local_event_store::node_events::NodeEventRow;
    use crate::domain::workflow::ExecutionParentRef;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Identity {
        node_execution_id: String,
        node_name: String,
        kind: String,
        attempt: u32,
        parent: Option<ExecutionParentRef>,
    }
    let mut parents = std::collections::HashMap::new();
    let rows = events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let identity: Identity = serde_json::from_value(event.payload.clone())
                .map_err(|error| WorkflowError::external(error.to_string()))?;
            if event.event_kind == "started" {
                parents.insert(
                    identity.node_execution_id.clone(),
                    identity.parent.map(|parent| parent.parent_id),
                );
            }
            Ok(NodeEventRow {
                tree_id: event.execution_id.clone(),
                parent_id: parents.get(&identity.node_execution_id).cloned().flatten(),
                node_execution_id: identity.node_execution_id,
                node_name: identity.node_name,
                kind: identity.kind,
                attempt: i64::from(identity.attempt),
                seq: index as i64 + 1,
                timestamp_ms: (event.timestamp * 1000.0) as i64,
                event_type: event.event_kind.clone(),
                session_id: None,
                detail: event.payload.to_string(),
            })
        })
        .collect::<Result<Vec<_>, WorkflowError>>()?;
    fact_log::records_from_tree_rows(&rows).map_err(WorkflowError::external)
}

#[cfg(test)]
#[path = "execution_projection_repository_test.rs"]
mod execution_projection_repository_tests;
