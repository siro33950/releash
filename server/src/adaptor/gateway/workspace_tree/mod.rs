pub(crate) mod execution_reads;
pub(crate) mod query_service;
pub(crate) mod repository;

pub(crate) use query_service::SqliteWorkspaceQueryService;
pub(crate) use repository::SqliteWorkspaceTreeRepository;

fn snapshot_nodes(
    workspace: &str,
    folded: &crate::domain::workflow::services::fact_replay::FoldedTree,
    record: &crate::domain::local_event::WorkflowExecutionMetadataRecord,
    processes: &Option<std::sync::Arc<dyn crate::domain::workflow::NodeProcessReader>>,
) -> Result<
    Vec<crate::domain::workspace_tree::WorkspaceTreeNode>,
    crate::domain::workflow::WorkflowError,
> {
    use crate::domain::workspace_tree::RuntimeSnapshotNodeProjection;
    let mut process_presences = std::collections::HashMap::new();
    if let Some(processes) = processes {
        for node in &folded.aggregate.node_executions {
            process_presences.insert(
                node.id.clone(),
                processes.presence(workspace, &node.id, node.kind, node.session_id.as_deref())?,
            );
        }
    }
    let delegate_waiting_node_ids = folded
        .aggregate
        .node_executions
        .iter()
        .filter(|node| folded.aggregate.delegate_waits_for_child(&node.id))
        .map(|node| node.id.clone())
        .collect();
    crate::domain::workspace_tree::runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
        command_failure_reasons: &folded.command_failure_reasons,
        process_presences: &process_presences,
        execution_id: &folded.aggregate.id,
        workflow_name: &folded.aggregate.workflow_name,
        workspace_identity: workspace,
        workflow_definition: folded.aggregate.workflow.as_ref(),
        node_executions: &folded.aggregate.node_executions,
        retry_predecessors: &folded.aggregate.retry_predecessors,
        delegate_waiting_node_ids: &delegate_waiting_node_ids,
        execution_active: folded.aggregate.is_active(),
        started_at: folded.aggregate.started_at,
        updated_at: folded.aggregate.updated_at,
        execution: record,

        session_activities: &folded.session_activities,
        session_display_names: &folded.session_display_names,
    })
    .map_err(|reason| record_projection_error(&reason))
}

fn record_projection_error(context: &str) -> crate::domain::workflow::WorkflowError {
    let correlation_id = uuid::Uuid::new_v4();
    log::error!("Workspace query record invariant failure [{correlation_id}]: {context}");
    crate::domain::workflow::WorkflowError::CorruptStoredState(format!(
        "{context} (correlation_id={correlation_id})"
    ))
}

fn background_failures(
    nodes: &[crate::domain::workspace_tree::WorkspaceTreeNode],
    failures: &dyn crate::domain::failure::FailureRecordRepository,
) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut targets = nodes
        .iter()
        .flat_map(|node| {
            [
                Some(node.id.as_str()),
                node.node_execution_id.as_deref(),
                node.execution_id.as_deref(),
            ]
        })
        .flatten()
        .rev()
        .filter(|target| seen.insert(*target))
        .collect::<Vec<_>>();
    targets.reverse();
    targets
        .into_iter()
        .flat_map(|target| {
            failures
                .attention_messages(target)
                .into_iter()
                .map(move |message| (target.to_string(), message))
        })
        .collect()
}
