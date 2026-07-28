use std::sync::Arc;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::workflow::{WorkflowError, WorkflowExecution, WorkflowExecutionId};
use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;

#[derive(Clone)]
pub(crate) struct WorkflowExecutionProjectionLogRepository {
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
}

impl WorkflowExecutionProjectionLogRepository {
    pub(crate) fn with_authority(
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    ) -> Self {
        Self {
            repository,
            installation_id,
        }
    }

    fn read_events(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEvent>, String> {
        WorkflowEventLog::with_authority(self.repository.clone(), self.installation_id.clone())
            .read_log_durable_blocking(execution_id.as_str())
    }
}

impl WorkflowExecutionProjectionRepository for WorkflowExecutionProjectionLogRepository {
    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        let events = self
            .read_events(execution_id)
            .map_err(WorkflowError::external)?;
        crate::domain::workflow::services::event_replay::project_workflow_execution(
            execution_id.as_str(),
            &events,
        )
        .map_err(WorkflowError::external)
    }
}
