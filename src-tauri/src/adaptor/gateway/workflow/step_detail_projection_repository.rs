use std::path::PathBuf;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::workflow::{RunId, WorkflowError};
use crate::usecase::workflow::ports::WorkflowStepDetailProjectionRepository;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowStepDetailProjectionLogRepository {
    data_dir: PathBuf,
}

impl WorkflowStepDetailProjectionLogRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }
}

impl WorkflowStepDetailProjectionRepository for WorkflowStepDetailProjectionLogRepository {
    fn get_step_detail(
        &self,
        run_id: &RunId,
        node_name: &str,
        run_index: Option<u32>,
    ) -> Result<Option<serde_json::Value>, WorkflowError> {
        let events = WorkflowEventLog::new(&self.data_dir)
            .read_log(run_id.as_str())
            .map_err(WorkflowError::external)?;
        let Some(state) =
            super::event_projection::reconstruct_state_from_events(run_id.as_str(), &events)
                .map_err(WorkflowError::external)?
        else {
            return Ok(None);
        };
        super::event_projection::compute_step_detail(&state, &events, node_name, run_index)
            .map(|detail| {
                serde_json::to_value(detail).map_err(|e| {
                    WorkflowError::external(format!("serialize workflow step detail: {e}"))
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::event::WorkflowEvent;
    use crate::adaptor::gateway::workflow::schema::{
        FacetRefs, NodeDefinition, NodeKind, NodeKindName, SessionSpec, Workflow,
    };
    use tempfile::TempDir;

    #[test]
    fn projects_persisted_events_to_step_detail_json() {
        let tmp = TempDir::new().unwrap();
        let run_id = RunId::new("00000000-0000-4000-8000-000000000302").unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            request: String::new(),
            workflow_definition: Workflow {
                name: "wf".to_string(),
                description: "test".to_string(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: "plan".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        facets: FacetRefs {
                            instruction: Some("plan it".to_string()),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..NodeDefinition::default()
                }],
            },
            timestamp: 10.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeStarted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "00000000-0000-4000-8000-000000000303".to_string(),
            node_name: "plan".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            fanout_parent: None,
            timestamp: 11.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            node_execution_id: "00000000-0000-4000-8000-000000000303".to_string(),
            node_name: "plan".to_string(),
            result: Some("done".to_string()),
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 12.0,
        })
        .unwrap();

        let detail = WorkflowStepDetailProjectionLogRepository::new(tmp.path())
            .get_step_detail(&run_id, "plan", Some(1))
            .unwrap()
            .unwrap();

        assert_eq!(detail["stepName"], "plan");
        assert_eq!(detail["runIndex"].as_u64(), Some(1));
        assert_eq!(detail["completedAtMs"].as_f64(), Some(12_000.0));
    }
}
