use std::collections::{HashMap, HashSet};

use super::{
    WorkspaceExecution, WorkspaceNodeKind, WorkspaceNodeStatusClassification, WorkspacePublicRoot,
    WorkspaceTree, WorkspaceTreeNode,
};
impl WorkspaceTree {
    pub(crate) fn active_execution_roots(
        &self,
    ) -> impl Iterator<Item = (&WorkspaceExecution, WorkspacePublicRoot<'_>)> {
        let mut roots = HashMap::new();
        let mut children = HashMap::<&str, Vec<&WorkspaceTreeNode>>::new();
        for node in self.nodes() {
            if node.kind == WorkspaceNodeKind::Workflow {
                if let Some(execution) = node.execution_id.as_deref() {
                    roots.entry(execution).or_insert(node);
                }
            }
            if let Some(parent) = node.parent_id.as_deref() {
                children.entry(parent).or_default().push(node);
            }
        }
        self.executions()
            .iter()
            .filter(|execution| execution.archive.is_none())
            .filter_map(move |execution| {
                let root = roots.get(execution.execution_id.as_str())?;
                Some((
                    execution,
                    WorkspacePublicRoot::for_owner(
                        children
                            .get(root.id.as_str())
                            .into_iter()
                            .flat_map(|nodes| nodes.iter().copied()),
                        root,
                    ),
                ))
            })
    }

    pub fn card_executions(&self) -> Vec<(&WorkspaceExecution, ExecutionNodes<'_>)> {
        let mut leaves = self.current_execution_leaves();
        let mut executions = self
            .active_execution_roots()
            .map(|(execution, root)| {
                (
                    execution,
                    ExecutionNodes {
                        root,
                        leaves: leaves
                            .remove(execution.execution_id.as_str())
                            .unwrap_or_default(),
                        launch: execution.launched_as,
                    },
                )
            })
            .collect::<Vec<_>>();
        executions.sort_by_key(|(_, nodes)| nodes.is_workflow());
        executions
    }
    fn current_execution_leaves(&self) -> HashMap<&str, Vec<&WorkspaceTreeNode>> {
        let nodes = self.nodes();
        let mut children = self.visible_children();
        let mut pending = children.remove(&None).unwrap_or_default();
        let mut visible = HashSet::new();
        while let Some(node) = pending.pop() {
            if node.is_leaf() {
                visible.insert(node.id.as_str());
            }
            pending.extend(children.remove(&Some(node.id.as_str())).unwrap_or_default());
        }
        let mut leaves = HashMap::<&str, Vec<&WorkspaceTreeNode>>::new();
        for node in nodes
            .iter()
            .filter(|node| visible.contains(node.id.as_str()))
        {
            if let Some(execution) = node.execution_id.as_deref() {
                leaves.entry(execution).or_default().push(node);
            }
        }
        leaves
    }
}

pub struct ExecutionNodes<'a> {
    root: WorkspacePublicRoot<'a>,
    pub leaves: Vec<&'a WorkspaceTreeNode>,
    launch: crate::domain::workflow::ExecutionTreeLaunch,
}
impl<'a> ExecutionNodes<'a> {
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
