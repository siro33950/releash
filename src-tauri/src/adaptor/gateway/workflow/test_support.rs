use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::LocalEventStore;

use super::event::WorkflowEvent;
use super::execution_store::WorkflowExecutionMetadata;

pub(crate) fn seed_canonical_execution(
    store: &Arc<LocalEventStore>,
    execution: &WorkflowExecutionMetadata,
    events: &[WorkflowEvent],
) {
    if events.is_empty() {
        // 事実列が既に seed 済みなら合成しない（既存の木を正とする）。
        let existing = super::fact_log::read_tree_records(store, &execution.execution_id).unwrap();
        if existing.is_empty() {
            let synthesized = synthesized_metadata_events(execution);
            super::fact_log::append_facts_for_events(store, &synthesized).unwrap();
        }
    } else {
        super::fact_log::append_facts_for_events(store, events).unwrap();
    }
}

/// metadata のみの seed を fold（node_events）でも観測できるよう、
/// 状態に対応する最小の事実列を合成する。
fn synthesized_metadata_events(execution: &WorkflowExecutionMetadata) -> Vec<WorkflowEvent> {
    use crate::domain::workflow::{
        ExecutionStatus, NodeDefinition, NodeKindName, WorkflowDefinition,
    };

    let definition = WorkflowDefinition {
        name: execution.workflow_name.clone(),
        description: String::new(),
        builtin: false,
        schemas: Default::default(),
        nodes: vec![NodeDefinition {
            name: "main".to_string(),
            kind: crate::domain::workflow::NodeKind::Session(
                crate::domain::workflow::SessionSpec::default(),
            ),
            artifact: None,
            input: Vec::new(),
            completion: crate::domain::workflow::NodeCompletion::Auto,
            worktree: None,
        }],
        entry: "main".to_string(),
    };
    let root_node_execution_id = format!("{}-root", execution.execution_id);
    let mut events = vec![
        WorkflowEvent::ExecutionStarted {
            execution_id: execution.execution_id.clone(),
            workflow_name: execution.workflow_name.clone(),
            worktree_path: execution.worktree_path.clone(),
            created_from: execution.created_from,
            request: String::new(),
            definition,
            timestamp: execution.started_at,
        },
        WorkflowEvent::NodeStarted {
            execution_id: execution.execution_id.clone(),
            node_execution_id: root_node_execution_id.clone(),
            node_name: "main".to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            parent: None,
            timestamp: execution.started_at,
        },
    ];
    let settled_at = execution.completed_at.unwrap_or(execution.updated_at);
    match execution.status {
        ExecutionStatus::Completed => {
            events.push(WorkflowEvent::NodeSubmitReceived {
                execution_id: execution.execution_id.clone(),
                node_execution_id: root_node_execution_id.clone(),
                timestamp: settled_at,
            });
            events.push(WorkflowEvent::NodeStopReceived {
                execution_id: execution.execution_id.clone(),
                node_execution_id: root_node_execution_id,
                timestamp: settled_at,
            });
        }
        ExecutionStatus::Aborted => {
            events.push(WorkflowEvent::ExecutionAborted {
                execution_id: execution.execution_id.clone(),
                aborted_node: None,
                timestamp: settled_at,
            });
        }
        _ => {}
    }
    events
}

pub(crate) fn append_canonical_events(
    store: &Arc<LocalEventStore>,
    events: &[WorkflowEvent],
) -> Result<(), String> {
    super::fact_log::append_facts_for_events(store, events)
}
