use std::path::PathBuf;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::workflow::{RunId, WorkflowError, WorkflowStateSnapshot};
use crate::usecase::workflow::ports::WorkflowStateProjectionRepository;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowStateProjectionLogRepository {
    data_dir: PathBuf,
}

impl WorkflowStateProjectionLogRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }
}

impl WorkflowStateProjectionRepository for WorkflowStateProjectionLogRepository {
    fn get_state(&self, run_id: &RunId) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        let events = WorkflowEventLog::new(&self.data_dir)
            .read_log(run_id.as_str())
            .map_err(WorkflowError::external)?;
        let state =
            super::event_projection::reconstruct_state_from_events(run_id.as_str(), &events)
                .map_err(WorkflowError::external)?;
        Ok(state.map(crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::Workflow;
    use tempfile::TempDir;

    #[test]
    fn projects_persisted_events_to_domain_state_snapshot() {
        let tmp = TempDir::new().unwrap();
        let run_id = RunId::new("00000000-0000-4000-8000-000000000301").unwrap();
        WorkflowEventLog::new(tmp.path())
            .append(&WorkflowEvent::RunStarted {
                run_id: run_id.to_string(),
                workflow_name: "wf".to_string(),
                workflow_file_stem: "wf".to_string(),
                worktree_path: "/repo".to_string(),
                workflow_definition: Workflow {
                    name: "wf".to_string(),
                    description: "test".to_string(),
                    builtin: false,
                    schemas: Default::default(),
                    variables: Default::default(),
                    nodes: vec![],
                },
                timestamp: 10.0,
            })
            .unwrap();

        let state = WorkflowStateProjectionLogRepository::new(tmp.path())
            .get_state(&run_id)
            .unwrap()
            .unwrap();

        assert_eq!(state.execution_id, run_id.to_string());
        assert_eq!(state.workflow_name, "wf");
        assert_eq!(state.started_at, 10.0);
    }
}
