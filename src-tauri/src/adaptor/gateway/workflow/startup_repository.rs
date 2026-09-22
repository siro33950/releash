use super::{fact_codec, fact_log, stored_definition};
use crate::adaptor::gateway::local_event_store::{node_events, LocalEventStore};
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::workflow::entities::workflow_execution::ExecutionTree;
use crate::domain::workflow::repository::{WorkflowStartupRecord, WorkflowStartupRepository};
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

    async fn is_registered_or_reserved(&self, tree_id: &str) -> bool {
        self.host
            .execution_tree_is_registered_or_reserved(tree_id)
            .await
    }

    async fn reconcile_tree(&self, tree_id: &str, timestamp: f64) -> Result<(), WorkflowError> {
        self.host
            .reconcile_tree(&self.app, tree_id, timestamp)
            .await
            .map_err(|error| WorkflowError::external(error.to_string()))
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
        let (first, terminal) = backend
            .run_indexed(move |connection| {
                let first = node_events::first_row_of_tree(connection, &requested)
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?;
                let terminal = first
                    .as_ref()
                    .map(|first| {
                        node_events::first_row_for_tree_with_event_types(
                            connection,
                            &first.tree_id,
                            fact_codec::terminal_event_types(),
                        )
                    })
                    .transpose()
                    .map_err(|_| LocalEventQueryError::InvalidRequest)?
                    .flatten();
                Ok((first, terminal))
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
        }))
    }

    fn append(
        &self,
        root: &NodeFactMeta,
        fact: &NodeFact,
        timestamp: f64,
    ) -> Result<(), WorkflowError> {
        fact_log::append_single_fact(&self.0, root, fact, (timestamp * 1000.0) as i64)
            .map_err(WorkflowError::external)
    }
}

#[cfg(test)]
#[path = "startup_repository_test.rs"]
mod startup_repository_tests;
