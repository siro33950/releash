use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::workflow::services::fact_replay;
use crate::domain::workflow::{WorkflowError, WorkflowExecution, WorkflowExecutionId};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowExecutionProjectionRepository};

/// 事実ログ（node_events）の tree fold から実行 read model を導出する。
#[derive(Clone)]
pub(crate) struct WorkflowExecutionProjectionLogRepository {
    backend: FactLogReadBackend,
}

impl WorkflowExecutionProjectionLogRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Self {
        Self {
            backend: FactLogReadBackend::Live(store),
        }
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            backend: FactLogReadBackend::ReadOnly(store),
        }
    }
}

impl WorkflowExecutionProjectionRepository for WorkflowExecutionProjectionLogRepository {
    fn get_node_artifact_from_events(
        &self,
        execution_id: &WorkflowExecutionId,
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
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        let records = fact_log::read_tree_records_from(&self.backend, execution_id.as_str())
            .map_err(WorkflowError::external)?;
        let Some(tree) = fact_replay::fold_execution_tree(execution_id.as_str(), &records)
            .map_err(WorkflowError::external)?
        else {
            return Ok(None);
        };
        Ok(Some(fact_replay::derive_read_model(&tree)))
    }
}

fn records_from_drafts(
    events: &[WorkflowEventDraft],
) -> Result<Vec<crate::domain::workflow::NodeFactRecord>, WorkflowError> {
    use crate::domain::workflow::{NodeFact, NodeFactMeta, NodeFactRecord, NodeKindName};
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Identity {
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
    }
    let mut parents = std::collections::HashMap::new();
    events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let identity: Identity = serde_json::from_value(event.payload.clone())
                .map_err(|error| WorkflowError::external(error.to_string()))?;
            let detail = event.payload.to_string();
            let Some(fact) = fact_log::decode_stored_fact(&event.event_kind, &detail)
                .map_err(WorkflowError::external)?
            else {
                return Ok(None);
            };
            if let NodeFact::Started(started) = &fact {
                parents.insert(
                    identity.node_execution_id.clone(),
                    started
                        .parent
                        .as_ref()
                        .map(|parent| parent.parent_id.clone()),
                );
            }
            Ok(Some(NodeFactRecord {
                meta: NodeFactMeta {
                    tree_id: event.execution_id.clone(),
                    parent_id: parents.get(&identity.node_execution_id).cloned().flatten(),
                    node_execution_id: identity.node_execution_id,
                    node_name: identity.node_name,
                    kind: identity.kind,
                    attempt: identity.attempt,
                },
                seq: index as i64 + 1,
                timestamp_ms: (event.timestamp * 1000.0) as i64,
                fact,
            }))
        })
        .filter_map(Result::transpose)
        .collect()
}

#[cfg(test)]
#[path = "execution_projection_repository_test.rs"]
mod execution_projection_repository_tests;
