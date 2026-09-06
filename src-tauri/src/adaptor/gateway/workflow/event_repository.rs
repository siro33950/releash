use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::node_events::{self, NodeEventRow};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend;
use crate::domain::workflow::{WorkflowError, WorkflowExecutionId, WorkflowPageRequest};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};

/// 事実ログ（node_events）を実行イベント一覧として読む repository。
///
/// event_kind は統一 Node の事実語彙（started / submit_received / ...）、
/// payload は detail カラムの JSON。
#[derive(Clone)]
pub(crate) struct WorkflowEventLogRepository {
    source: WorkflowEventReadSource,
}

#[derive(Clone)]
enum WorkflowEventReadSource {
    Canonical(FactLogReadBackend),
}

impl WorkflowEventLogRepository {
    pub(crate) fn with_store(store: Arc<LocalEventStore>) -> Self {
        Self {
            source: WorkflowEventReadSource::Canonical(FactLogReadBackend::Live(store)),
        }
    }

    pub(crate) fn with_read_store(store: Arc<LocalEventReadStore>) -> Self {
        Self {
            source: WorkflowEventReadSource::Canonical(FactLogReadBackend::ReadOnly(store)),
        }
    }

    fn read_drafts(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        match &self.source {
            WorkflowEventReadSource::Canonical(backend) => {
                let execution_id = execution_id.as_str().to_string();
                let rows = backend
                    .run_indexed(move |connection| {
                        node_events::read_tree(connection, &execution_id).map_err(|_| {
                            crate::domain::local_event::LocalEventQueryError::InvalidRequest
                        })
                    })
                    .map_err(|error| WorkflowError::external(error.to_string()))?;
                rows.iter().map(row_to_draft).collect()
            }
        }
    }

    fn read_draft_page(
        &self,
        execution_id: &WorkflowExecutionId,
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
                        .map_err(|_| {
                            crate::domain::local_event::LocalEventQueryError::InvalidRequest
                        })
                    })
                    .map_err(|error| WorkflowError::external(error.to_string()))?;
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

impl WorkflowEventRepository for WorkflowEventLogRepository {
    #[cfg(test)]
    fn append(&self, _event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        Err(WorkflowError::external(
            "canonical event repository is read-only",
        ))
    }

    fn read(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_drafts(execution_id)
    }

    fn read_page(
        &self,
        execution_id: &WorkflowExecutionId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.read_draft_page(execution_id, page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::domain::workflow::{
        ExecutionOrigin, NodeCompletion, NodeDefinition, NodeKind, NodeKindName, SessionSpec,
        WorkflowDefinition, WorkflowEvent,
    };

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "main".to_string(),
                kind: NodeKind::Session(SessionSpec::default()),
                artifact: None,
                input: Vec::new(),
                completion: NodeCompletion::Auto,
                worktree: None,
            }],
            entry: "main".to_string(),
        }
    }

    fn started_events(execution_id: &str) -> Vec<WorkflowEvent> {
        vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "ship it".to_string(),
                definition: definition(),
                timestamp: 1.0,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: format!("{execution_id}-root"),
                node_name: "main".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                parent: None,
                timestamp: 1.0,
            },
        ]
    }

    #[test]
    fn read_returns_fact_rows_with_unified_vocabulary() {
        let tmp = TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").unwrap();
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &store,
            &started_events(execution_id.as_str()),
        )
        .unwrap();
        let repo = WorkflowEventLogRepository::with_store(store);

        let events = repo.read(&execution_id).unwrap();

        assert_eq!(events[0].event_kind, "started");
        assert_eq!(events[0].payload["root"]["definition"]["name"], "wf");
        assert_eq!(events[0].payload["root"]["launchedAs"], "workflow");
        assert_eq!(events[0].payload["root"]["request"], "ship it");
    }

    #[test]
    fn read_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(tmp.path().to_path_buf()))
                .unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000002").unwrap();
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &store,
            &started_events(execution_id.as_str()),
        )
        .unwrap();
        let repo = WorkflowEventLogRepository::with_store(store.clone());

        // ExecutionStarted と root の NodeStarted は 1 つの root started 行に融合される。
        let first = repo.read(&execution_id).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event_kind, "started");

        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &store,
            &[WorkflowEvent::ExecutionAborted {
                execution_id: execution_id.to_string(),
                aborted_node: None,
                timestamp: 2.0,
            }],
        )
        .unwrap();

        let second = repo.read(&execution_id).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0], first[0]);
        assert_eq!(second[1].event_kind, "abort_requested");
        assert_eq!(second[1].timestamp, 2.0);
    }
    #[test]
    fn test_実行履歴_未対応定義を落とさず保存されたpayloadをページでも返す() {
        // Given
        let directory = TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let id = WorkflowExecutionId::new("00000000-0000-4000-8000-000000001744").unwrap();
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &store,
            id.as_str(),
            "/repo",
            "main",
        );
        let repository = WorkflowEventLogRepository::with_store(store);

        // When
        let records = repository.read(&id).unwrap();
        let page = repository
            .read_page(
                &id,
                WorkflowPageRequest {
                    offset: 0,
                    limit: 1,
                },
            )
            .unwrap();

        // Then
        assert_eq!(
            records[0].payload["root"]["definition"]["nodes"]["main"]["sequence"]["output"],
            "session"
        );
        assert_eq!(page, records[..1]);
    }
}
