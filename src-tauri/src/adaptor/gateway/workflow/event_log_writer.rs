use std::path::Path;

use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;

pub(crate) fn append_required_events(
    data_dir: &Path,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    WorkflowEventLog::new(data_dir).append_batch(events)
}

pub(crate) fn append_required_events_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
        .map_err(|_| "failed to resolve app data dir".to_string())?;
    append_required_events(&data_dir, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_required_events_writes_batch_to_execution_log() {
        let tmp = tempfile::tempdir().unwrap();
        let execution_id = "00000000-0000-0000-0000-000000000001";
        let event = WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: None,
            timestamp: 42.0,
        };

        append_required_events(tmp.path(), std::slice::from_ref(&event)).unwrap();

        let events = WorkflowEventLog::new(tmp.path())
            .read_log(execution_id)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            WorkflowEvent::ExecutionAborted {
                execution_id: restored_execution_id,
                timestamp,
                ..
            } if restored_execution_id == execution_id
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn append_required_events_delegates_empty_batch_as_noop() {
        let tmp = tempfile::tempdir().unwrap();

        append_required_events(tmp.path(), &[]).unwrap();

        assert!(!tmp.path().join("workflow_execution_logs").exists());
    }
}
