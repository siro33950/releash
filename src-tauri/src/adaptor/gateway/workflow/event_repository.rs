use std::path::PathBuf;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::workflow::{RunId, WorkflowError};
use crate::usecase::workflow::ports::{WorkflowEventDraft, WorkflowEventRepository};

use super::mapper;

#[derive(Debug, Clone)]
pub(crate) struct WorkflowEventLogRepository {
    data_dir: PathBuf,
}

impl WorkflowEventLogRepository {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    fn log(&self) -> WorkflowEventLog {
        WorkflowEventLog::new(&self.data_dir)
    }
}

impl WorkflowEventRepository for WorkflowEventLogRepository {
    #[cfg(test)]
    fn append(&self, event: &WorkflowEventDraft) -> Result<(), WorkflowError> {
        self.append_batch(std::slice::from_ref(event))
    }

    #[cfg(test)]
    fn append_batch(&self, events: &[WorkflowEventDraft]) -> Result<(), WorkflowError> {
        let legacy_events: Vec<_> = events
            .iter()
            .map(mapper::domain_event_draft_to_legacy)
            .collect::<Result<_, _>>()?;
        self.log()
            .append_batch(&legacy_events)
            .map_err(WorkflowError::external)
    }

    fn read(&self, run_id: &RunId) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.log()
            .read_log(run_id.as_str())
            .map_err(WorkflowError::external)?
            .iter()
            .map(mapper::legacy_event_to_domain_draft)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_preserves_existing_ndjson_event_tag() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let run_id = RunId::new("00000000-0000-4000-8000-000000000001").unwrap();

        repo.append(&WorkflowEventDraft {
            run_id: run_id.to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflowName": "wf",
                "workflowFileStem": "wf",
                "worktreePath": "/repo",
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "type": "agent"
                    }]
                },
                "permissionMode": "edit"
            }),
        })
        .unwrap();

        let content = std::fs::read_to_string(
            tmp.path()
                .join("workflow_logs")
                .join(format!("{run_id}.ndjson")),
        )
        .unwrap();
        assert!(content.contains("\"event\":\"run_started\""));

        let events = repo.read(&run_id).unwrap();
        assert_eq!(events[0].event_kind, "run_started");
        assert_eq!(events[0].payload["workflow_name"], "wf");
    }

    #[test]
    fn read_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let run_id = RunId::new("00000000-0000-4000-8000-000000000002").unwrap();

        repo.append(&WorkflowEventDraft {
            run_id: run_id.to_string(),
            event_kind: "run_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflowName": "wf",
                "workflowFileStem": "wf",
                "worktreePath": "/repo",
                "workflowDefinition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "type": "agent"
                    }]
                },
                "permissionMode": "edit"
            }),
        })
        .unwrap();

        let first = repo.read(&run_id).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event_kind, "run_started");
        assert_eq!(first[0].payload["workflow_name"], "wf");

        repo.append(&WorkflowEventDraft {
            run_id: run_id.to_string(),
            event_kind: "run_aborted".to_string(),
            timestamp: 2.0,
            payload: serde_json::json!({
                "workflowName": "wf",
            }),
        })
        .unwrap();

        let second = repo.read(&run_id).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0], first[0]);
        assert_eq!(second[1].event_kind, "run_aborted");
        assert_eq!(second[1].timestamp, 2.0);
        assert_eq!(second[1].payload["workflow_name"], "wf");
    }
}
