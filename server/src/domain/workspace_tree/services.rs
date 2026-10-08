use std::collections::HashSet;

use super::entities::WorkspaceTree;
use super::value_objects::WorkspaceNodeKind;
use super::WorkspaceTreeNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspacePublicRoot<'a> {
    owner: &'a WorkspaceTreeNode,
    node: &'a WorkspaceTreeNode,
}

impl<'a> WorkspacePublicRoot<'a> {
    pub fn all(nodes: &'a [WorkspaceTreeNode]) -> Vec<Self> {
        nodes
            .iter()
            .filter(|node| node.kind == WorkspaceNodeKind::Workflow)
            .filter_map(|owner| Self::from_owner(nodes, owner))
            .collect()
    }

    pub fn for_execution(nodes: &'a [WorkspaceTreeNode], execution_id: &str) -> Option<Self> {
        Self::all(nodes)
            .into_iter()
            .find(|root| root.public_id() == execution_id)
    }

    pub fn for_node(nodes: &'a [WorkspaceTreeNode], node_id: &str) -> Option<Self> {
        Self::all(nodes)
            .into_iter()
            .find(|root| root.node.id == node_id)
    }

    pub fn owner(&self) -> &'a WorkspaceTreeNode {
        self.owner
    }

    pub fn node(&self) -> &'a WorkspaceTreeNode {
        self.node
    }

    pub fn public_id(&self) -> &'a str {
        self.owner
            .execution_id
            .as_deref()
            .expect("a Workflow root owner must have an execution id")
    }

    pub fn public_title(&self) -> &'a str {
        if self.node.is_standalone_session_root() {
            self.node.title.as_str()
        } else {
            self.owner.title.as_str()
        }
    }

    fn from_owner(nodes: &'a [WorkspaceTreeNode], owner: &'a WorkspaceTreeNode) -> Option<Self> {
        owner.execution_id.as_ref()?;
        let node = nodes
            .iter()
            .filter(|node| {
                node.parent_id.as_deref() == Some(owner.id.as_str())
                    && !node.is_internal_rule_record()
                    && !node.is_retry_history
            })
            .min_by_key(|node| (node.sibling_order, node.id.as_str()))?;
        Some(Self { owner, node })
    }
}

pub struct WorkspaceTreeVisibilityPolicy;

impl WorkspaceTreeVisibilityPolicy {
    pub fn hidden_branch_ids<'a>(
        tree: &'a WorkspaceTree,
        active_archive_execution_ids: impl IntoIterator<Item = &'a str>,
    ) -> HashSet<String> {
        let archived = active_archive_execution_ids
            .into_iter()
            .collect::<HashSet<_>>();
        tree.nodes()
            .iter()
            .filter(|node| {
                node.kind == WorkspaceNodeKind::Workflow
                    && node
                        .execution_id
                        .as_deref()
                        .is_some_and(|execution_id| archived.contains(execution_id))
            })
            .map(|node| node.id.clone())
            .collect()
    }
}

#[cfg(test)]
#[path = "services_test.rs"]
pub(crate) mod services_tests;
