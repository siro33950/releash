use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::LocalEventStore;
use crate::domain::local_event::{
    CommitOperationKind, LocalEventTransactionRepository, LocalStateMutation, Revision,
    RevisionGuard, SessionProjectionMutation, SessionProjectionRecord,
    WorkflowExecutionMetadataRecord, WorkflowExecutionNodeProjectionMutation,
    WorkflowExecutionProjectionMutation, WorkflowExecutionProjectionRecord,
};

use super::event::WorkflowEvent;
use super::execution_store::WorkflowExecutionMetadata;
use super::log::WorkflowEventLog;

fn event_log(store: &Arc<LocalEventStore>) -> WorkflowEventLog {
    let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
    WorkflowEventLog::with_authority(repository, store.installation_id().to_string())
}

pub(crate) fn seed_canonical_execution(
    store: &Arc<LocalEventStore>,
    execution: &WorkflowExecutionMetadata,
    events: &[WorkflowEvent],
) {
    let record = WorkflowExecutionMetadataRecord {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        worktree_path: execution.worktree_path.clone(),
        current_node: execution.current_node.clone(),
        created_from: execution.created_from,
        started_at_bits: execution.started_at.to_bits(),
        updated_at_bits: execution.updated_at.to_bits(),
        completed_at_bits: execution.completed_at.map(f64::to_bits),
        error_reason: execution.error_reason.clone(),
        interruption_reason: execution.interruption_reason,
        resume_from_node: execution.resume_from_node.clone(),
        total_token_usage: execution.total_token_usage.clone(),
    };
    let mutations = vec![
        LocalStateMutation::SessionProjection(SessionProjectionMutation {
            session_id: format!("workflow:{}", execution.execution_id),
            projection: SessionProjectionRecord::WorkflowExecution(
                WorkflowExecutionProjectionRecord::Present(record.clone()),
            ),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
        LocalStateMutation::WorkflowExecutionProjection(WorkflowExecutionProjectionMutation {
            projection: WorkflowExecutionProjectionRecord::Present(record),
            expected: RevisionGuard::Absent,
            revision: Revision::new(0).unwrap(),
        }),
        LocalStateMutation::WorkflowExecutionNodeProjection(
            WorkflowExecutionNodeProjectionMutation {
                execution_id: execution.execution_id.clone(),
                nodes: Vec::new(),
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            },
        ),
    ];
    let log = event_log(store);
    if events.is_empty() {
        log.commit_projection_durable_blocking(&execution.execution_id, mutations)
            .unwrap();
    } else {
        log.append_batch_durable_with_mutations_blocking_as(
            CommitOperationKind::Workflow,
            events,
            mutations,
        )
        .unwrap();
    }
}

pub(crate) fn append_canonical_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    event_log(store).append_batch_durable_with_mutations_blocking_as(
        CommitOperationKind::Workflow,
        events,
        Vec::new(),
    )
}
