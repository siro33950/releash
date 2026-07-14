use std::path::PathBuf;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::workflow::{WorkflowError, WorkflowExecution, WorkflowExecutionId};
use crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowExecutionProjectionLogRepository {
    data_dir: PathBuf,
}

impl WorkflowExecutionProjectionLogRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }
}

impl WorkflowExecutionProjectionRepository for WorkflowExecutionProjectionLogRepository {
    fn get_execution(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Option<WorkflowExecution>, WorkflowError> {
        let events = WorkflowEventLog::new(&self.data_dir)
            .read_log(execution_id.as_str())
            .map_err(WorkflowError::external)?;
        super::event_projection::project_workflow_execution(execution_id.as_str(), &events)
            .map_err(WorkflowError::external)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::Workflow;
    use crate::domain::workflow::ExecutionOrigin;

    #[test]
    fn projects_persisted_events_to_the_canonical_read_model() {
        let temp = tempfile::tempdir().unwrap();
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000301").unwrap();
        WorkflowEventLog::new(temp.path())
            .append(&WorkflowEvent::ExecutionStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::Cli,
                request: "review this".to_string(),
                definition: Workflow {
                    name: "review".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: Vec::new(),
                },
                timestamp: 10.0,
            })
            .unwrap();

        let execution = WorkflowExecutionProjectionLogRepository::new(temp.path())
            .get_execution(&execution_id)
            .unwrap()
            .unwrap();

        assert_eq!(execution.id, execution_id.to_string());
        assert_eq!(execution.workflow_name, "review");
        assert_eq!(execution.started_at, 10.0);
        assert_eq!(execution.artifacts[0].node_name, "request");
    }
}
