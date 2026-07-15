use std::path::PathBuf;

use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::domain::workflow::{WorkflowError, WorkflowExecutionId, WorkflowPageRequest};
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
        let workflow_events: Vec<_> = events
            .iter()
            .map(mapper::event_draft_to_event)
            .collect::<Result<_, _>>()?;
        self.log()
            .append_batch(&workflow_events)
            .map_err(WorkflowError::external)
    }

    fn read(
        &self,
        execution_id: &WorkflowExecutionId,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.log()
            .read_log(execution_id.as_str())
            .map_err(WorkflowError::external)?
            .iter()
            .map(mapper::workflow_event_to_domain_draft)
            .collect()
    }

    fn read_page(
        &self,
        execution_id: &WorkflowExecutionId,
        page: WorkflowPageRequest,
    ) -> Result<Vec<WorkflowEventDraft>, WorkflowError> {
        self.log()
            .read_log_page(execution_id.as_str(), page.offset, page.limit)
            .map_err(WorkflowError::external)?
            .iter()
            .map(mapper::workflow_event_to_domain_draft)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn append_preserves_canonical_execution_event_tag() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000001").unwrap();

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "ship it",
                "permission_mode": "ask",
                "definition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "session": { "gate": "auto" }
                    }]
                }
            }),
        })
        .unwrap();

        let content = std::fs::read_to_string(
            tmp.path()
                .join("workflow_execution_logs")
                .join(format!("{execution_id}.ndjson")),
        )
        .unwrap();
        assert!(content.contains("\"event\":\"execution_started\""));

        let events = repo.read(&execution_id).unwrap();
        assert_eq!(events[0].event_kind, "execution_started");
        assert_eq!(events[0].payload["workflow_name"], "wf");
        assert_eq!(events[0].payload["request"], "ship it");
    }

    #[test]
    fn read_after_cached_read_observes_incremental_append() {
        let tmp = TempDir::new().unwrap();
        let repo = WorkflowEventLogRepository::new(tmp.path());
        let execution_id =
            WorkflowExecutionId::new("00000000-0000-4000-8000-000000000002").unwrap();

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 1.0,
            payload: serde_json::json!({
                "workflow_name": "wf",
                "worktree_path": "/repo",
                "created_from": "cli",
                "request": "",
                "permission_mode": "ask",
                "definition": {
                    "name": "wf",
                    "description": "",
                    "nodes": [{
                        "name": "step",
                        "session": { "gate": "auto" }
                    }]
                }
            }),
        })
        .unwrap();

        let first = repo.read(&execution_id).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].event_kind, "execution_started");
        assert_eq!(first[0].payload["workflow_name"], "wf");

        repo.append(&WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_aborted".to_string(),
            timestamp: 2.0,
            payload: serde_json::json!({"aborted_node": null}),
        })
        .unwrap();

        let second = repo.read(&execution_id).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0], first[0]);
        assert_eq!(second[1].event_kind, "execution_aborted");
        assert_eq!(second[1].timestamp, 2.0);
        assert_eq!(second[1].payload["aborted_node"], serde_json::Value::Null);
    }
}
