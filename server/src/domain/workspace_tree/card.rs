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
                self.nodes()
                    .iter()
                    .find(|node| {
                        node.kind == WorkspaceNodeKind::Workflow
                            && node.execution_id.as_deref() == Some(execution.execution_id.as_str())
                    })
                    .map(|root| (execution, root))
            })
    }
    pub fn card_executions(&self) -> Vec<(&WorkspaceExecution, ExecutionNodes<'_>)> {
        let mut executions = self
            .active_execution_roots()
            .map(|(execution, root)| (execution, ExecutionNodes::with_root(root, self.nodes())))
            .collect::<Vec<_>>();
        executions.sort_by_key(|(execution, _)| ExecutionNodes::is_workflow(execution.launched_as));
        executions
    }
    pub fn card_status(&self) -> Option<WorkspaceNodeStatusClassification> {
        self.active_execution_roots()
            .map(|(_, root)| root.status_classification)
            .reduce(WorkspaceNodeStatusClassification::most_severe)
    }
}

pub struct ExecutionNodes<'a> {
    pub root: &'a WorkspaceTreeNode,
    pub leaves: Vec<&'a WorkspaceTreeNode>,
}
impl<'a> ExecutionNodes<'a> {
    fn with_root(root: &'a WorkspaceTreeNode, nodes: &'a [WorkspaceTreeNode]) -> Self {
        let leaves = nodes
            .iter()
            .filter(|node| {
                node.execution_id == root.execution_id && !node.is_retry_history && node.is_leaf()
            })
            .collect();
        Self { root, leaves }
    }
    pub fn is_workflow(launch: crate::domain::workflow::ExecutionTreeLaunch) -> bool {
        launch == crate::domain::workflow::ExecutionTreeLaunch::Workflow
    }
    pub fn title_node(
        &self,
        launch: crate::domain::workflow::ExecutionTreeLaunch,
    ) -> &WorkspaceTreeNode {
        if Self::is_workflow(launch) {
            self.root
        } else {
            self.leaves.first().copied().unwrap_or(self.root)
        }
    }
    pub fn session_states(&self) -> impl Iterator<Item = WorkspaceNodeStatusClassification> + '_ {
        self.leaves
            .iter()
            .filter(|node| node.kind == WorkspaceNodeKind::WorkflowSession)
            .map(|node| node.status_classification)
    }
    pub fn node_count(&self) -> usize {
        self.leaves.len()
    }
}

#[cfg(test)]
#[path = "card_test.rs"]
mod card_tests;
