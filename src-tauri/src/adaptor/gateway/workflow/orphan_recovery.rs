use crate::adaptor::gateway::workflow::event::WorkflowEvent;
use crate::adaptor::gateway::workflow::run::WorkflowRun;

#[derive(Debug, Clone)]
pub(crate) struct OrphanRunRecoveryItem {
    pub(crate) run_id: String,
    pub(crate) run: WorkflowRun,
    pub(crate) event: WorkflowEvent,
    pub(crate) completed_at: f64,
}

pub(crate) fn orphan_run_recovery_items(
    orphans: Vec<WorkflowRun>,
    timestamp: f64,
) -> Vec<OrphanRunRecoveryItem> {
    orphans
        .into_iter()
        .map(|run| {
            let run_id = run.run_id.clone();
            let event = WorkflowEvent::RunAborted {
                run_id: run_id.clone(),
                workflow_name: run.workflow_name.clone(),
                timestamp,
            };
            OrphanRunRecoveryItem {
                run_id,
                run,
                event,
                completed_at: timestamp,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::run::{RunStatus, TriggerSource};

    fn run(run_id: &str, workflow_name: &str) -> WorkflowRun {
        WorkflowRun {
            run_id: run_id.to_string(),
            workflow_name: workflow_name.to_string(),
            task: Some("task".to_string()),
            status: RunStatus::Running,
            worktree_path: "/tmp/wt".to_string(),
            current_node_name: Some("step".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
        }
    }

    #[test]
    fn orphan_run_recovery_items_builds_run_aborted_event_for_each_orphan() {
        let items =
            orphan_run_recovery_items(vec![run("run-1", "wf-1"), run("run-2", "wf-2")], 42.0);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].run_id, "run-1");
        assert_eq!(items[0].completed_at, 42.0);
        assert_eq!(items[0].run.workflow_name, "wf-1");
        assert!(matches!(
            &items[0].event,
            WorkflowEvent::RunAborted {
                run_id,
                workflow_name,
                timestamp,
            } if run_id == "run-1"
                && workflow_name == "wf-1"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            &items[1].event,
            WorkflowEvent::RunAborted {
                run_id,
                workflow_name,
                timestamp,
            } if run_id == "run-2"
                && workflow_name == "wf-2"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn orphan_run_recovery_items_returns_empty_when_no_orphans() {
        assert!(orphan_run_recovery_items(Vec::new(), 42.0).is_empty());
    }
}
