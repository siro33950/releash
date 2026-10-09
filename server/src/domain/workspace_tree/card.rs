use super::{
    WorkspaceExecution, WorkspaceNodeKind, WorkspaceNodeStatusClassification, WorkspaceTree,
    WorkspaceTreeNode,
};
impl WorkspaceTree {
    pub(crate) fn active_execution_roots(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceExecution, &WorkspaceTreeNode)> {
        self.executions()
            .iter()
            .filter(|execution| execution.archive.is_none())
            .filter_map(|execution| {
                ExecutionNodes::new(&execution.execution_id, self.nodes())
                    .map(|nodes| (execution, nodes.root))
            })
    }
    pub fn card_status(&self) -> Option<WorkspaceNodeStatusClassification> {
        self.active_execution_roots()
            .map(|(_, root)| root.status_classification)
            .reduce(WorkspaceNodeStatusClassification::most_severe)
    }
}

pub(crate) struct ExecutionNodes<'a> {
    pub root: &'a WorkspaceTreeNode,
    pub leaves: Vec<&'a WorkspaceTreeNode>,
}
impl<'a> ExecutionNodes<'a> {
    pub fn new(execution: &str, nodes: &'a [WorkspaceTreeNode]) -> Option<Self> {
        let root = nodes.iter().find(|node| {
            node.kind == WorkspaceNodeKind::Workflow
                && node.execution_id.as_deref() == Some(execution)
        })?;
        let leaves = nodes
            .iter()
            .filter(|node| {
                node.execution_id.as_deref() == Some(execution)
                    && !node.is_retry_history
                    && node.is_leaf()
            })
            .collect();
        Some(Self { root, leaves })
    }
    pub fn is_workflow(launch: crate::domain::workflow::ExecutionTreeLaunch) -> bool {
        launch == crate::domain::workflow::ExecutionTreeLaunch::Workflow
    }
    pub fn node_count(&self) -> usize {
        self.leaves.len()
    }
}
pub(crate) fn observe_node_failures(
    nodes: &mut [WorkspaceTreeNode],
    failures: &dyn crate::domain::failure::FailureRecordRepository,
) {
    for node in nodes.iter_mut() {
        for target in [
            Some(node.id.as_str()),
            node.node_execution_id.as_deref(),
            node.execution_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        {
            for message in failures.attention_messages(&target) {
                node.observe_background_failure(&message);
            }
        }
    }
    super::entities::aggregate_node_status_classifications(nodes);
}
