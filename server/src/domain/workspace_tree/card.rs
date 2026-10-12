use std::collections::{HashMap, HashSet};

use super::latest_attempts::{AttemptScope, LatestAttempts};
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
        let latest = LatestAttempts::new(nodes.iter().filter_map(|node| {
            Some((
                AttemptScope {
                    execution: node.execution_id.as_deref()?,
                    name: node.node_name.as_deref()?,
                    parent: node.execution_parent.as_ref(),
                },
                node.attempt?,
            ))
        }));
        let by_id = nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<HashMap<_, _>>();
        let mut children = HashMap::<&str, Vec<&WorkspaceTreeNode>>::new();
        let mut pending = Vec::new();
        for node in nodes {
            if let Some(parent) = node
                .parent_id
                .as_deref()
                .filter(|id| by_id.contains_key(id))
            {
                children.entry(parent).or_default().push(node);
            } else {
                pending.push(node);
            }
        }
        let mut visible = HashSet::new();
        let mut visited = HashSet::new();
        while let Some(node) = pending.pop() {
            if !visited.insert(node.id.as_str()) {
                continue;
            }
            let current = !node.is_retry_history
                && match (
                    node.execution_id.as_deref(),
                    node.node_name.as_deref(),
                    node.attempt,
                ) {
                    (Some(execution), Some(name), Some(attempt)) => latest.contains(
                        AttemptScope {
                            execution,
                            name,
                            parent: node.execution_parent.as_ref(),
                        },
                        attempt,
                    ),
                    _ => true,
                };
            if current && node.is_leaf() {
                visible.insert(node.id.as_str());
            }
            if current || node.is_leaf() {
                pending.extend(children.remove(node.id.as_str()).unwrap_or_default());
            }
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
