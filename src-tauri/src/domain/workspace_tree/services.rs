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
mod tests {
    use super::*;
    use crate::domain::workflow::NodeCompletionSignalState;
    use crate::domain::workspace_tree::WorkspaceNodeStatus;

    fn node(
        id: &str,
        parent_id: Option<&str>,
        sibling_order: u64,
        kind: WorkspaceNodeKind,
        execution_id: Option<&str>,
    ) -> WorkspaceTreeNode {
        WorkspaceTreeNode {
            id: id.to_string(),
            parent_id: parent_id.map(str::to_string),
            sibling_order,
            kind,
            title: id.to_string(),
            status: WorkspaceNodeStatus::Running,
            error_reason: None,
            updated_at_bits: 0.0_f64.to_bits(),
            execution_id: execution_id.map(str::to_string),
            node_execution_id: None,
            node_name: None,
            attempt: None,
            retry_predecessor_id: None,
            past_attempt_ids: Vec::new(),
            is_retry_history: false,
            completion_signals: NodeCompletionSignalState::Pending,
            has_artifact: false,
            session_id: None,
            can_approve: false,
            can_retry: false,
            can_close: false,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        }
    }

    #[test]
    fn public_root_is_the_first_visible_direct_child_of_each_workflow_owner() {
        let mut retry = node(
            "retry-history",
            Some("owner-a"),
            0,
            WorkspaceNodeKind::WorkflowSession,
            Some("execution-a"),
        );
        retry.is_retry_history = true;
        let nodes = vec![
            node(
                "owner-a",
                None,
                0,
                WorkspaceNodeKind::Workflow,
                Some("execution-a"),
            ),
            retry,
            node(
                "second",
                Some("owner-a"),
                2,
                WorkspaceNodeKind::WorkflowCommand,
                Some("execution-a"),
            ),
            node(
                "first",
                Some("owner-a"),
                1,
                WorkspaceNodeKind::Sequence,
                Some("execution-a"),
            ),
            node(
                "owner-b",
                None,
                1,
                WorkspaceNodeKind::Workflow,
                Some("execution-b"),
            ),
            node(
                "session-root",
                Some("owner-b"),
                0,
                WorkspaceNodeKind::WorkflowSession,
                Some("execution-b"),
            ),
        ];

        let roots = WorkspacePublicRoot::all(&nodes);

        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].owner().id, "owner-a");
        assert_eq!(roots[0].node().id, "first");
        assert_eq!(roots[0].public_id(), "execution-a");
        assert_eq!(roots[1].node().id, "session-root");
        assert_eq!(roots[1].public_id(), "execution-b");
    }

    #[test]
    fn public_root_lookups_share_the_same_resolved_relationship() {
        let nodes = vec![
            node(
                "owner",
                None,
                0,
                WorkspaceNodeKind::Workflow,
                Some("execution"),
            ),
            node(
                "root",
                Some("owner"),
                0,
                WorkspaceNodeKind::Fanout,
                Some("execution"),
            ),
            node(
                "child",
                Some("owner"),
                1,
                WorkspaceNodeKind::WorkflowCommand,
                Some("execution"),
            ),
        ];

        let by_execution = WorkspacePublicRoot::for_execution(&nodes, "execution").unwrap();
        let by_node = WorkspacePublicRoot::for_node(&nodes, "root").unwrap();

        assert_eq!(by_execution, by_node);
        assert!(WorkspacePublicRoot::for_node(&nodes, "child").is_none());
    }
}
