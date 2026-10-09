use crate::domain::workflow::ExecutionTreeLaunch;
use crate::domain::workspace_tree::{WorkspaceNodeKind, WorkspaceTreeNode};
use crate::usecase::workspace_tree::query_service::WorktreeExecutionSummary;

pub(crate) fn execution_summary(
    id: &str,
    name: &str,
    launch: ExecutionTreeLaunch,
    provider: Option<crate::domain::provider_lifecycle::ProviderKind>,
    nodes: &[WorkspaceTreeNode],
) -> Option<WorktreeExecutionSummary> {
    let nodes = crate::domain::workspace_tree::card::ExecutionNodes::new(id, nodes)?;
    let is_workflow = crate::domain::workspace_tree::card::ExecutionNodes::is_workflow(launch);
    Some(WorktreeExecutionSummary {
        id: id.into(),
        title: if is_workflow {
            name.into()
        } else {
            nodes
                .leaves
                .first()
                .map_or_else(|| name.into(), |node| node.title.clone())
        },
        is_workflow,
        provider,
        status: nodes.root.status_classification,
        node_count: nodes.node_count(),
        session_states: nodes
            .leaves
            .into_iter()
            .filter(|node| node.kind == WorkspaceNodeKind::WorkflowSession)
            .map(|node| node.status_classification)
            .collect(),
    })
}
#[cfg(test)]
#[path = "execution_summary_test.rs"]
mod execution_summary_tests;
