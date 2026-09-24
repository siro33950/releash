use super::{fact_codec, fact_log, stored_definition};
use crate::adaptor::gateway::local_event_store::writer::NodeEventWriteError;
use crate::adaptor::gateway::local_event_store::{node_events, LocalEventStore};
use crate::domain::failure::ClassifiedFailure;
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
use crate::domain::workflow::repository::{
    WorkflowRevision, WorkflowStartupRecord, WorkflowStartupRepository,
};
use crate::domain::workflow::{NodeFact, NodeFactMeta, WorkflowError};
use std::sync::Arc;

pub struct StoredWorkflowStartupRepository(pub Arc<LocalEventStore>);

pub(crate) struct HostWorkflowStartup {
    pub(crate) host: Arc<super::workflow_host::WorkflowRuntimeHost>,
    pub(crate) app: super::workflow_host::WorkflowRuntimeDependencies,
}

#[async_trait::async_trait]
impl crate::usecase::workflow::startup::WorkflowStartupGateway for HostWorkflowStartup {
    fn current_timestamp(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64())
    }

    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError> {
        self.host
            .reconcile_tree(&self.app, tree_id, timestamp)
            .await
            .map_err(|error| match error {
                crate::usecase::workflow::runtime_error::WorkflowRuntimeError::Conflict(reason) => {
                    WorkflowError::Conflict(reason)
                }
                error => WorkflowError::external(error.to_string()),
            })
    }
}

impl WorkflowStartupRepository for StoredWorkflowStartupRepository {
    fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        fact_log::list_tree_ids(&fact_log::FactLogReadBackend::Live(self.0.clone()), None)
            .map_err(WorkflowError::external)
    }

    fn load(&self, tree_id: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        let backend = fact_log::FactLogReadBackend::Live(self.0.clone());
        let requested = tree_id.to_string();
        let (first, terminal, head) = backend
            .run_indexed(move |connection| {
                let transaction = connection
                    .unchecked_transaction()
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?;
                let first = node_events::first_row_of_tree(&transaction, &requested)
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?;
                let terminal = first
                    .as_ref()
                    .map(|first| {
                        node_events::first_row_for_tree_with_event_types(
                            &transaction,
                            &first.tree_id,
                            fact_codec::terminal_event_types(),
                        )
                    })
                    .transpose()
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?
                    .flatten();
                let head = transaction
                    .query_row(
                        "SELECT COALESCE(MAX(seq), 0) FROM node_events WHERE tree_id = ?1",
                        [&requested],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?;
                Ok((first, terminal, head))
            })
            .map_err(|error| {
                WorkflowError::external(format!("node fact tree read failed: {error:?}"))
            })?;
        let Some(first) = first else {
            return Ok(None);
        };
        if first.event_type != "started" {
            return Err(WorkflowError::external(format!(
                "tree {tree_id} does not begin with a started fact"
            )));
        }
        let NodeFact::Started(started) = stored_definition::decode_terminal_started(&first.detail)
            .map_err(WorkflowError::external)?
        else {
            unreachable!()
        };
        let root = started
            .root
            .ok_or_else(|| WorkflowError::external("tree root fact is missing"))?;
        let mut execution = ExecutionTree::restore_without_definition(
            tree_id,
            &root,
            first.timestamp_ms as f64 / 1000.0,
        );
        if let Some(row) = terminal {
            let fact = fact_codec::decode(&row.event_type, &row.detail)
                .map_err(|error| WorkflowError::external(error.to_string()))?;
            execution.replay_terminal_fact(&fact, row.timestamp_ms as f64 / 1000.0);
        }
        let definition_error = if execution.is_active() {
            stored_definition::definition_error(&first.detail).map_err(WorkflowError::external)?
        } else {
            None
        };
        Ok(Some(WorkflowStartupRecord {
            execution,
            root: fact_log::node_meta_from_row(&first).map_err(WorkflowError::external)?,
            definition_error,
            revision: WorkflowRevision(head.to_string()),
        }))
    }

    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
        expected_revision: Option<&WorkflowRevision>,
    ) -> Result<(), WorkflowError> {
        let expected_head = expected_revision
            .map(|revision| revision.0.parse::<i64>())
            .transpose()
            .map_err(|_| WorkflowError::invalid_state("invalid workflow revision"))?;
        let pending = fact_log::pending_single_fact(root, fact, (timestamp * 1000.0) as i64)
            .map_err(WorkflowError::external)?;
        let result = self.0.append_node_events_at_head_blocking(
            vec![(pending.row.clone(), Some(pending.timestamp_ms))],
            expected_head.map(|head| (root.tree_id.clone(), head)),
        );
        match result {
            Err(NodeEventWriteError::OutcomeUnknown) => {
                match fact_log::resolve_unknown_append(&self.0, vec![pending], expected_head)
                    .map_err(|error| WorkflowError::StorageUnavailable {
                        kind: error.failure_kind(),
                        message: format!("startup abort readback failed: {error:?}"),
                    })? {
                    Ok(_) => Ok(()),
                    Err(NodeEventWriteError::Conflict | NodeEventWriteError::OutcomeUnknown) => {
                        Err(WorkflowError::Conflict(format!(
                            "tree {} startup abort was not found after an unknown append outcome",
                            root.tree_id
                        )))
                    }
                    Err(error) => Err(WorkflowError::from(error)),
                }
            }
            result => result.map(|_| ()).map_err(|error| match error {
                NodeEventWriteError::Conflict => WorkflowError::Conflict(format!(
                    "tree {} advanced before startup abort",
                    root.tree_id
                )),
                error => WorkflowError::from(error),
            }),
        }
    }
}

#[cfg(test)]
#[path = "startup_repository_test.rs"]
mod startup_repository_tests;
