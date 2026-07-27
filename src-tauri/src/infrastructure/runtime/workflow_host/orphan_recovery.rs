//! Per-execution orphan recovery planning.

use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
use crate::domain::workflow::ExecutionInterruptionReason;
use crate::domain::workflow::WorkflowEvent;

#[derive(Debug, Clone)]
pub(crate) struct OrphanExecutionRecoveryItem {
    pub(crate) execution_id: String,
    pub(crate) metadata: WorkflowExecutionMetadata,
    pub(crate) event: WorkflowEvent,
}

pub(crate) fn orphan_execution_recovery_items(
    orphans: Vec<WorkflowExecutionMetadata>,
    timestamp: f64,
) -> Vec<OrphanExecutionRecoveryItem> {
    orphans
        .into_iter()
        .map(|metadata| {
            let execution_id = metadata.execution_id.clone();
            let event = WorkflowEvent::ExecutionInterrupted {
                execution_id: execution_id.clone(),
                reason: ExecutionInterruptionReason::Orphan,
                timestamp,
            };
            OrphanExecutionRecoveryItem {
                execution_id,
                metadata,
                event,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};

    fn execution_metadata(execution_id: &str, workflow_name: &str) -> WorkflowExecutionMetadata {
        WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/tmp/wt".to_string(),
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: Default::default(),
        }
    }

    #[test]
    fn orphan_recovery_items_build_execution_interrupted_event_for_each_orphan() {
        let items = orphan_execution_recovery_items(
            vec![
                execution_metadata("execution-1", "wf-1"),
                execution_metadata("execution-2", "wf-2"),
            ],
            42.0,
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].execution_id, "execution-1");
        assert_eq!(items[0].metadata.workflow_name, "wf-1");
        assert!(matches!(
            &items[0].event,
            WorkflowEvent::ExecutionInterrupted {
                execution_id,
                reason: ExecutionInterruptionReason::Orphan,
                timestamp,
            } if execution_id == "execution-1"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            &items[1].event,
            WorkflowEvent::ExecutionInterrupted {
                execution_id,
                reason: ExecutionInterruptionReason::Orphan,
                timestamp,
            } if execution_id == "execution-2"
                && (*timestamp - 42.0).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn orphan_execution_recovery_items_returns_empty_when_no_orphans() {
        assert!(orphan_execution_recovery_items(Vec::new(), 42.0).is_empty());
    }
}
