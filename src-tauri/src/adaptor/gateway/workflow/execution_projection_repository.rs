use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::workflow::services::fact_replay;
use crate::domain::workflow::{WorkflowError, WorkflowExecution, WorkflowExecutionId};
use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;

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
