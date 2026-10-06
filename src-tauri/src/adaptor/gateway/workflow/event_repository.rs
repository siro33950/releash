use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::node_events::{self, NodeEventRow};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend;
use crate::domain::workflow::{ExecutionTreeId, WorkflowError, WorkflowPageRequest};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};

/// 事実ログ（node_events）を実行イベント一覧として読む repository。
///
/// event_kind は統一 Node の事実語彙（started / submit_received / ...）、
/// payload は detail カラムの JSON。
#[derive(Clone)]
pub struct WorkflowEventLogRepository {
    source: WorkflowEventReadSource,
}

#[derive(Clone)]
enum WorkflowEventReadSource {
    Canonical(FactLogReadBackend),
}

impl WorkflowEventLogRepository {
    pub fn with_store(store: Arc<LocalEventStore>) -> Self {
        Self {
            source: WorkflowEventReadSource::Canonical(FactLogReadBackend::Live(store)),
        }
    }

    pub(crate) fn with_read_store(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            source: WorkflowEventReadSource::Canonical(FactLogReadBackend::ReadOnly(store)),
        }
    }

    async fn read_drafts(
        &self,
        execution_id: &ExecutionTreeId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        match &self.source {
            WorkflowEventReadSource::Canonical(backend) => {
                let execution_id = execution_id.as_str().to_string();
                let rows = backend
                    .run_indexed(move |connection| {
                        node_events::read_tree(connection, &execution_id).map_err(|error| {
                            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                                &error,
                            )
                        })
                    })
                    .await
                    .map_err(|error| {
                        WorkflowError::from(super::fact_log::FactReadError::Query(error))
                    })?;
                rows.iter().map(row_to_draft).collect()
            }
        }
    }

    async fn read_draft_page(
        &self,
        execution_id: &ExecutionTreeId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        match &self.source {
            WorkflowEventReadSource::Canonical(backend) => {
                let execution_id = execution_id.as_str().to_string();
                let rows = backend
                    .run_indexed(move |connection| {
                        node_events::read_tree_page(
                            connection,
                            &execution_id,
                            page.offset,
                            page.limit,
                        )
                        .map_err(|error| {
                            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                                &error,
                            )
                        })
                    })
                    .await
                    .map_err(|error| {
                        WorkflowError::from(super::fact_log::FactReadError::Query(error))
                    })?;
                rows.iter().map(row_to_draft).collect()
            }
        }
    }
}

fn row_to_draft(row: &NodeEventRow) -> Result<WorkflowEventDraft, WorkflowError> {
    let mut payload: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&row.detail)
        .map_err(|error| WorkflowError::external(error.to_string()))?;
    payload.insert(
        "nodeExecutionId".to_string(),
        row.node_execution_id.clone().into(),
    );
    payload.insert("nodeName".to_string(), row.node_name.clone().into());
    payload.insert("kind".to_string(), row.kind.clone().into());
    payload.insert("attempt".to_string(), row.attempt.into());
    Ok(WorkflowEventDraft {
        execution_id: row.tree_id.clone(),
        event_kind: row.event_type.clone(),
        timestamp: row.timestamp_ms as f64 / 1000.0,
        payload: serde_json::Value::Object(payload),
    })
}

#[async_trait::async_trait]
impl WorkflowEventRepository for WorkflowEventLogRepository {
    #[cfg(test)]
    fn append(&self, _event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        Err(WorkflowError::external(
            "canonical event repository is read-only",
        ))
    }

    async fn read(
        &self,
        execution_id: &ExecutionTreeId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_drafts(execution_id).await
    }

    async fn read_page(
        &self,
        execution_id: &ExecutionTreeId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_draft_page(execution_id, page).await
    }
}
