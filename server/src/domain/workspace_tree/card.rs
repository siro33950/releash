use super::{
    WorkspaceExecution, WorkspaceNodeKind, WorkspaceNodeStatusClassification, WorkspacePublicRoot,
    WorkspaceTree, WorkspaceTreeNode,
};
impl WorkspaceTree {
    pub(crate) fn active_execution_roots(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceExecution, WorkspacePublicRoot<'_>)> {
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
                    .map(|root| {
                        (
                            execution,
                            WorkspacePublicRoot::for_owner(self.nodes(), root),
                        )
                    })
            })
    }
    pub fn card_executions(&self) -> Vec<(&WorkspaceExecution, ExecutionNodes<'_>)> {
        let mut executions = self
            .active_execution_roots()
            .map(|(execution, root)| {
                (
                    execution,
                    ExecutionNodes::with_root(root, self.nodes(), execution.launched_as),
                )
            })
            .collect::<Vec<_>>();
        executions.sort_by_key(|(_, nodes)| nodes.is_workflow());
        executions
    }
    pub fn card_status(&self) -> Option<WorkspaceNodeStatusClassification> {
        self.active_execution_roots()
            .map(|(execution, root)| {
                ExecutionNodes::node_for(root, execution.launched_as).status_classification
            })
            .reduce(WorkspaceNodeStatusClassification::most_severe)
    }
}

pub struct ExecutionNodes<'a> {
    root: WorkspacePublicRoot<'a>,
    pub leaves: Vec<&'a WorkspaceTreeNode>,
    launch: crate::domain::workflow::ExecutionTreeLaunch,
}
impl<'a> ExecutionNodes<'a> {
    fn with_root(
        root: WorkspacePublicRoot<'a>,
        nodes: &'a [WorkspaceTreeNode],
        launch: crate::domain::workflow::ExecutionTreeLaunch,
    ) -> Self {
        let current = nodes
            .iter()
            .filter(|node| node.execution_id == root.owner().execution_id)
            .filter(|node| {
                !node.is_retry_history
                    && !nodes.iter().any(|candidate| {
                        candidate.execution_id == node.execution_id
                            && candidate.node_name == node.node_name
                            && candidate.execution_parent == node.execution_parent
                            && candidate.attempt > node.attempt
                    })
            })
            .map(|node| node.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let leaves = nodes
            .iter()
            .filter(|node| {
                node.execution_id == root.owner().execution_id && node.is_leaf() && {
                    let mut ancestor = Some(*node);
                    while let Some(ancestor_node) = ancestor {
                        if !current.contains(ancestor_node.id.as_str())
                            && (ancestor_node.id == node.id || !ancestor_node.is_leaf())
                        {
                            break;
                        }
                        ancestor = ancestor_node
                            .parent_id
                            .as_ref()
                            .and_then(|parent| nodes.iter().find(|node| &node.id == parent));
                    }
                    ancestor.is_none()
                }
            })
            .collect();
        Self {
            root,
            leaves,
            launch,
        }
    }
    pub fn is_workflow(&self) -> bool {
        self.launch == crate::domain::workflow::ExecutionTreeLaunch::Workflow
    }
    fn node_for(
        root: WorkspacePublicRoot<'a>,
        launch: crate::domain::workflow::ExecutionTreeLaunch,
    ) -> &'a WorkspaceTreeNode {
        match launch {
            crate::domain::workflow::ExecutionTreeLaunch::Session => root.node(),
            crate::domain::workflow::ExecutionTreeLaunch::Workflow => root.owner(),
        }
    }
    pub fn node(&self) -> &'a WorkspaceTreeNode {
        Self::node_for(self.root, self.launch)
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
