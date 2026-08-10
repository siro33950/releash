//! Durable Workspace tree aggregate and projector.
//!
//! This aggregate owns structural participation, parentage, sibling order,
//! opaque identities, and the one-to-one Node/Session bindings. Persistence
//! records and public DTOs deliberately live outside this module.

use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use super::value_objects::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus, WorkspaceSessionFact,
    WorkspaceSessionState, WorkspaceStructureFact, WorkspaceTreeError, WorkspaceTreeNode,
    INTERNAL_SIBLING_ORDER,
};
use crate::domain::workflow::{
    ExecutionStatus, FanoutParentRef, ItemsSource, NodeCompletionSignalState,
    NodeExecutionFailureKind, NodeKindName, WorkflowDefinition,
};

const DEFAULT_SESSION_TITLE: &str = "NewSession";
pub(super) const DEFAULT_WORKFLOW_TITLE: &str = "Workflow";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTree {
    workspace_identity: WorkspaceIdentity,
    nodes: Vec<WorkspaceTreeNode>,
}

impl WorkspaceTree {
    pub fn empty(workspace_identity: impl Into<String>) -> Self {
        Self {
            workspace_identity: WorkspaceIdentity::new(workspace_identity.into()),
            nodes: Vec::new(),
        }
    }

    pub fn restore(
        workspace_identity: impl Into<String>,
        nodes: Vec<WorkspaceTreeNode>,
    ) -> Result<Self, WorkspaceTreeError> {
        let mut tree = Self {
            workspace_identity: WorkspaceIdentity::new(workspace_identity.into()),
            nodes,
        };
        tree.recompute_root_order();
        tree.recompute_branch_summaries();
        tree.validate()?;
        Ok(tree)
    }

    pub fn nodes(&self) -> &[WorkspaceTreeNode] {
        &self.nodes
    }

    pub fn preferred_node_id(&self, hidden: &HashSet<String>) -> Option<String> {
        let mut children = BTreeMap::<Option<&str>, Vec<&WorkspaceTreeNode>>::new();
        for node in &self.nodes {
            if !node.is_internal_rule_record() {
                children
                    .entry(node.parent_id.as_deref())
                    .or_default()
                    .push(node);
            }
        }
        for siblings in children.values_mut() {
            siblings.sort_by_key(|node| (node.sibling_order, node.id.as_str()));
        }
        fn collect<'a>(
            parent: Option<&str>,
            children: &BTreeMap<Option<&str>, Vec<&'a WorkspaceTreeNode>>,
            hidden: &HashSet<String>,
            leaves: &mut Vec<&'a WorkspaceTreeNode>,
        ) {
            for node in children.get(&parent).into_iter().flatten() {
                if hidden.contains(&node.id) {
                    continue;
                }
                if node.is_leaf() {
                    leaves.push(node);
                } else {
                    collect(Some(&node.id), children, hidden, leaves);
                }
            }
        }
        let mut leaves = Vec::new();
        collect(None, &children, hidden, &mut leaves);
        leaves
            .iter()
            .find(|node| {
                matches!(
                    node.status,
                    WorkspaceNodeStatus::Running | WorkspaceNodeStatus::Waiting
                )
            })
            .or_else(|| leaves.first())
            .map(|node| node.id.clone())
    }

    pub fn session_node(&self, session_id: &str) -> Option<&WorkspaceTreeNode> {
        self.nodes
            .iter()
            .find(|node| node.session_id.as_deref() == Some(session_id))
    }

    pub(super) fn session_node_mut(&mut self, session_id: &str) -> Option<&mut WorkspaceTreeNode> {
        self.nodes
            .iter_mut()
            .find(|node| node.session_id.as_deref() == Some(session_id))
    }

    pub(super) fn validate(&self) -> Result<(), WorkspaceTreeError> {
        let ids = self
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        if ids.len() != self.nodes.len() {
            let duplicate = self
                .nodes
                .iter()
                .find(|candidate| {
                    self.nodes
                        .iter()
                        .filter(|node| node.id == candidate.id)
                        .count()
                        > 1
                })
                .map(|node| node.id.clone())
                .unwrap_or_default();
            return Err(WorkspaceTreeError::DuplicateNode(duplicate));
        }
        let mut sessions = HashSet::new();
        let mut node_executions = HashSet::new();
        let mut workflow_executions = HashSet::new();
        let mut sibling_orders = HashSet::new();
        for node in &self.nodes {
            if node.id.is_empty()
                || node.title.is_empty()
                || !node.updated_at().is_finite()
                || !node_shape_is_valid(node)
            {
                return Err(WorkspaceTreeError::InvalidNode(node.id.clone()));
            }
            if let Some(parent) = &node.parent_id {
                let Some(parent_node) = self.nodes.iter().find(|candidate| &candidate.id == parent)
                else {
                    return Err(WorkspaceTreeError::MissingParent(parent.clone()));
                };
                if !matches!(
                    parent_node.kind,
                    WorkspaceNodeKind::Workflow | WorkspaceNodeKind::Fanout
                ) {
                    return Err(WorkspaceTreeError::InvalidParent(parent.clone()));
                }
                if parent_node.execution_id != node.execution_id {
                    return Err(WorkspaceTreeError::InvalidParent(parent.clone()));
                }
            }
            if let Some(session_id) = &node.session_id {
                if !sessions.insert(session_id) {
                    return Err(WorkspaceTreeError::DuplicateSession(session_id.clone()));
                }
                if !node.is_leaf() {
                    return Err(WorkspaceTreeError::InvalidParent(node.id.clone()));
                }
            }
            if !node.is_internal_rule_record()
                && !sibling_orders.insert((node.parent_id.as_deref(), node.sibling_order))
            {
                return Err(WorkspaceTreeError::DuplicateSiblingOrder(node.id.clone()));
            }
            if let Some(node_execution_id) = &node.node_execution_id {
                if !node_executions.insert(node_execution_id) {
                    return Err(WorkspaceTreeError::DuplicateNodeExecution(
                        node_execution_id.clone(),
                    ));
                }
            }
            if node.kind == WorkspaceNodeKind::Workflow {
                let execution_id = node
                    .execution_id
                    .as_ref()
                    .ok_or_else(|| WorkspaceTreeError::InvalidNode(node.id.clone()))?;
                if !workflow_executions.insert(execution_id) {
                    return Err(WorkspaceTreeError::DuplicateNode(execution_id.clone()));
                }
            }
            let mut ancestors = HashSet::new();
            let mut cursor = node.parent_id.as_deref();
            while let Some(parent) = cursor {
                if parent == node.id || !ancestors.insert(parent) {
                    return Err(WorkspaceTreeError::ParentCycle(node.id.clone()));
                }
                cursor = self
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == parent)
                    .and_then(|candidate| candidate.parent_id.as_deref());
            }
        }
        Ok(())
    }

    fn next_sibling_order(&self, parent_id: Option<&str>) -> u64 {
        self.nodes
            .iter()
            .filter(|node| {
                node.parent_id.as_deref() == parent_id && !node.is_internal_rule_record()
            })
            .map(|node| node.sibling_order)
            .max()
            .and_then(|value| value.checked_add(1))
            .unwrap_or(0)
    }

    pub(super) fn workflow_node(&self, execution_id: &str) -> Option<&WorkspaceTreeNode> {
        self.nodes.iter().find(|node| {
            node.kind == WorkspaceNodeKind::Workflow
                && node.execution_id.as_deref() == Some(execution_id)
        })
    }

    pub(super) fn workflow_node_mut(
        &mut self,
        execution_id: &str,
    ) -> Option<&mut WorkspaceTreeNode> {
        self.nodes.iter_mut().find(|node| {
            node.kind == WorkspaceNodeKind::Workflow
                && node.execution_id.as_deref() == Some(execution_id)
        })
    }

    pub(super) fn execution_node_mut(
        &mut self,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Option<&mut WorkspaceTreeNode> {
        self.nodes.iter_mut().find(|node| {
            node.execution_id.as_deref() == Some(execution_id)
                && node.node_execution_id.as_deref() == Some(node_execution_id)
        })
    }

    pub(super) fn remove_subtree(&mut self, root_id: &str) {
        let mut removed = HashSet::from([root_id.to_string()]);
        loop {
            let before = removed.len();
            for node in &self.nodes {
                if node
                    .parent_id
                    .as_ref()
                    .is_some_and(|parent| removed.contains(parent))
                {
                    removed.insert(node.id.clone());
                }
            }
            if removed.len() == before {
                break;
            }
        }
        self.nodes.retain(|node| !removed.contains(&node.id));
    }

    pub(super) fn apply_session(
        &mut self,
        session: WorkspaceSessionFact,
    ) -> Result<(), WorkspaceTreeError> {
        if WorkspaceIdentity::new(&session.worktree_path) != self.workspace_identity {
            return Err(WorkspaceTreeError::IdentityMismatch);
        }
        if !session.workflow_node_session
            && matches!(
                session.state,
                WorkspaceSessionState::Closed | WorkspaceSessionState::Archived
            )
        {
            if let Some(node) = self.session_node(&session.id).cloned() {
                if node.kind == WorkspaceNodeKind::Session {
                    self.remove_subtree(&node.id);
                }
            }
            return Ok(());
        }
        if session.workflow_node_session {
            if let Some(node) = self.nodes.iter_mut().find(|node| {
                (node
                    .node_execution_id
                    .as_deref()
                    .zip(session.workflow_node_execution_id.as_deref())
                    .is_some_and(|(left, right)| left == right)
                    && node.execution_id.as_deref() == session.workflow_execution_id.as_deref())
                    || node.session_id.as_deref() == Some(session.id.as_str())
            }) {
                if node
                    .session_id
                    .as_ref()
                    .is_some_and(|bound| bound != &session.id)
                {
                    return Err(WorkspaceTreeError::DuplicateSession(session.id));
                }
                node.session_id = Some(session.id);
                if session.unresolved_recovery_reason.is_some() {
                    node.recovery_owner_reason = session.unresolved_recovery_reason;
                }
                node.updated_at_bits = max_f64_bits(node.updated_at_bits, session.updated_at_bits);
            }
            return Ok(());
        }

        let id = WorkspaceTreeProjector::session_node_id(&session.id);
        let title = session
            .title
            .filter(|title| !title.trim().is_empty())
            .or_else(|| {
                let first_message = session.first_message.trim();
                (!first_message.is_empty()).then(|| first_message.to_string())
            })
            .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());
        let status = session_status(session.state);
        let can_close = !matches!(
            session.state,
            WorkspaceSessionState::Closed | WorkspaceSessionState::Archived
        );
        if let Some(node) = self.nodes.iter_mut().find(|node| node.id == id) {
            node.title = title;
            node.status = status;
            node.error_reason = session.error_reason;
            node.updated_at_bits = session.updated_at_bits;
            node.session_id = Some(session.id);
            node.can_close = can_close;
            return Ok(());
        }
        let sibling_order = self.next_sibling_order(None);
        self.nodes.push(WorkspaceTreeNode {
            id,
            parent_id: None,
            sibling_order,
            kind: WorkspaceNodeKind::Session,
            title,
            status,
            error_reason: session.error_reason,
            updated_at_bits: session.updated_at_bits,
            execution_id: None,
            node_execution_id: None,
            node_name: None,
            attempt: None,
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: Some(session.id),
            can_approve: false,
            can_retry: false,
            can_close,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        });
        Ok(())
    }

    pub(super) fn apply_workflow_started(
        &mut self,
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        definition: WorkflowDefinition,
        timestamp: f64,
    ) -> Result<(), WorkspaceTreeError> {
        if WorkspaceIdentity::new(&worktree_path) != self.workspace_identity {
            return Err(WorkspaceTreeError::IdentityMismatch);
        }
        if self.workflow_node(&execution_id).is_some() {
            return Ok(());
        }
        let dynamic_fanout_names = definition
            .nodes
            .iter()
            .filter_map(|node| {
                node.fanout()
                    .filter(|fanout| {
                        matches!(fanout.items, Some(ItemsSource::ArtifactField { .. }))
                    })
                    .map(|_| node.name.clone())
            })
            .collect::<HashSet<_>>();
        let sibling_order = self.next_sibling_order(None);
        self.nodes.push(WorkspaceTreeNode {
            id: execution_id.clone(),
            parent_id: None,
            sibling_order,
            kind: WorkspaceNodeKind::Workflow,
            title: non_empty_or(workflow_name, DEFAULT_WORKFLOW_TITLE),
            status: WorkspaceNodeStatus::Running,
            error_reason: None,
            updated_at_bits: timestamp.to_bits(),
            execution_id: Some(execution_id.clone()),
            node_execution_id: None,
            node_name: None,
            attempt: None,
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: None,
            can_approve: false,
            can_retry: false,
            can_close: false,
            can_stop: true,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: true,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        });
        // These compact rule records are aggregate state, not visible Nodes.
        // They preserve dynamic/static fanout identity across later commits
        // without retaining the full Workflow definition.
        for name in dynamic_fanout_names {
            self.nodes.push(WorkspaceTreeNode {
                id: dynamic_fanout_sentinel_id(&execution_id, &name),
                parent_id: Some(execution_id.clone()),
                sibling_order: INTERNAL_SIBLING_ORDER,
                kind: WorkspaceNodeKind::Fanout,
                title: name,
                status: WorkspaceNodeStatus::Waiting,
                error_reason: None,
                updated_at_bits: timestamp.to_bits(),
                execution_id: Some(execution_id.clone()),
                node_execution_id: None,
                node_name: None,
                attempt: None,
                completion_signals: Default::default(),
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
                dynamic_fanout: true,
            });
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_node_started(
        &mut self,
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        timestamp: f64,
    ) -> Result<(), WorkspaceTreeError> {
        if self
            .execution_node_mut(&execution_id, &node_execution_id)
            .is_some()
        {
            return Ok(());
        }
        let workflow_id = self
            .workflow_node(&execution_id)
            .map(|node| node.id.clone())
            .ok_or_else(|| WorkspaceTreeError::MissingWorkflow(execution_id.clone()))?;

        let (parent_id, semantic_key, dynamic_fanout) = if let Some(parent) = fanout_parent {
            let fanout = self
                .nodes
                .iter()
                .find(|node| {
                    node.kind == WorkspaceNodeKind::Fanout
                        && !node.is_internal_rule_record()
                        && node.execution_id.as_deref() == Some(execution_id.as_str())
                        && node.node_name.as_deref() == Some(parent.parent_node.as_str())
                        && node.attempt == Some(parent.parent_attempt)
                })
                .ok_or_else(|| WorkspaceTreeError::MissingParent(parent.parent_node.clone()))?;
            let prior = self
                .nodes
                .iter()
                .filter(|node| {
                    node.parent_id.as_deref() == Some(fanout.id.as_str())
                        && node.node_name.as_deref() == Some(node_name.as_str())
                })
                .count();
            let semantic_key = if fanout.dynamic_fanout {
                fanout_dynamic_child_occurrence_key(
                    &parent.parent_node,
                    workflow_occurrence(&self.nodes, &fanout.id),
                    parent.child_index,
                    &node_name,
                    prior,
                )
            } else {
                fanout_child_occurrence_key(
                    &parent.parent_node,
                    workflow_occurrence(&self.nodes, &fanout.id),
                    parent.item_index,
                    parent.child_index,
                    &node_name,
                    prior,
                )
            };
            (fanout.id.clone(), semantic_key, fanout.dynamic_fanout)
        } else {
            let occurrence = self
                .nodes
                .iter()
                .filter(|node| {
                    node.parent_id.as_deref() == Some(workflow_id.as_str())
                        && node.node_name.as_deref() == Some(node_name.as_str())
                        && node.node_execution_id.is_some()
                })
                .count();
            let semantic_key = match kind {
                NodeKindName::Fanout => {
                    fanout_branch_occurrence_key(&execution_id, &node_name, occurrence)
                }
                NodeKindName::Session | NodeKindName::Command => {
                    workflow_node_occurrence_key(&node_name, occurrence)
                }
            };
            let dynamic = kind == NodeKindName::Fanout
                && self
                    .nodes
                    .iter()
                    .any(|node| node.id == dynamic_fanout_sentinel_id(&execution_id, &node_name));
            (workflow_id, semantic_key, dynamic)
        };
        let sibling_order = self.next_sibling_order(Some(&parent_id));
        let id = match kind {
            NodeKindName::Fanout => opaque_branch_id(&semantic_key),
            NodeKindName::Session | NodeKindName::Command => {
                opaque_workflow_node_id(&execution_id, &semantic_key)?
            }
        };
        self.nodes.push(WorkspaceTreeNode {
            id,
            parent_id: Some(parent_id),
            sibling_order,
            kind: match kind {
                NodeKindName::Fanout => WorkspaceNodeKind::Fanout,
                NodeKindName::Session => WorkspaceNodeKind::WorkflowSession,
                NodeKindName::Command => WorkspaceNodeKind::WorkflowCommand,
            },
            title: node_name.clone(),
            status: WorkspaceNodeStatus::Running,
            error_reason: None,
            updated_at_bits: timestamp.to_bits(),
            execution_id: Some(execution_id),
            node_execution_id: Some(node_execution_id),
            node_name: Some(node_name),
            attempt: Some(attempt),
            completion_signals: Default::default(),
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
            dynamic_fanout,
        });
        Ok(())
    }

    pub(super) fn recompute_branch_summaries(&mut self) {
        let mut by_parent = BTreeMap::<String, Vec<usize>>::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.is_internal_rule_record() {
                if let Some(parent) = &node.parent_id {
                    by_parent.entry(parent.clone()).or_default().push(index);
                }
            }
        }
        let mut branch_ids = self
            .nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.kind,
                    WorkspaceNodeKind::Fanout | WorkspaceNodeKind::Workflow
                ) && !node.is_internal_rule_record()
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        branch_ids.reverse();
        for branch_id in branch_ids {
            let Some(children) = by_parent.get(&branch_id) else {
                continue;
            };
            let child_status = children
                .iter()
                .map(|index| self.nodes[*index].status)
                .collect::<Vec<_>>();
            let child_updated = children
                .iter()
                .map(|index| self.nodes[*index].updated_at_bits)
                .max_by(|left, right| f64::from_bits(*left).total_cmp(&f64::from_bits(*right)));
            let current_child_status = children
                .iter()
                .max_by_key(|index| self.nodes[**index].sibling_order)
                .map(|index| self.nodes[*index].status);
            if let Some(branch) = self.nodes.iter_mut().find(|node| node.id == branch_id) {
                if branch.kind == WorkspaceNodeKind::Fanout {
                    branch.status = aggregate_status(&child_status, branch.status);
                } else if branch.kind == WorkspaceNodeKind::Workflow
                    && !branch.can_archive
                    && branch.status != WorkspaceNodeStatus::Interrupted
                {
                    branch.status = match current_child_status {
                        Some(WorkspaceNodeStatus::Completed) | None => WorkspaceNodeStatus::Running,
                        Some(status) => status,
                    };
                }
                if let Some(updated) = child_updated {
                    branch.updated_at_bits = max_f64_bits(branch.updated_at_bits, updated);
                }
            }
        }
        self.recompute_workflow_recovery_capabilities();
    }

    fn recompute_workflow_recovery_capabilities(&mut self) {
        let execution_ids = self
            .nodes
            .iter()
            .filter(|node| node.kind == WorkspaceNodeKind::Workflow)
            .filter_map(|node| node.execution_id.clone())
            .collect::<Vec<_>>();
        for execution_id in execution_ids {
            let Some(workflow) = self.workflow_node(&execution_id) else {
                continue;
            };
            let interrupted = workflow.status == WorkspaceNodeStatus::Interrupted;
            let execution_reason = workflow.recovery_owner_reason.clone();
            let waiting_approval = self.nodes.iter().any(|node| {
                node.execution_id.as_deref() == Some(execution_id.as_str()) && node.can_approve
            });
            let mut session_reasons = self
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == WorkspaceNodeKind::WorkflowSession
                        && node.execution_id.as_deref() == Some(execution_id.as_str())
                })
                .filter_map(|node| {
                    let session_id = node.session_id.as_ref()?;
                    let reason = node
                        .recovery_owner_reason
                        .clone()
                        .or_else(|| execution_reason.clone())?;
                    Some((session_id.clone(), reason))
                })
                .collect::<Vec<_>>();
            session_reasons.sort_by(|left, right| left.0.cmp(&right.0));
            let paused = self.nodes.iter().any(|node| {
                node.execution_id.as_deref() == Some(execution_id.as_str())
                    && node.is_leaf()
                    && node.status == WorkspaceNodeStatus::Paused
            });
            let reason = (interrupted || paused)
                .then(|| {
                    session_reasons
                        .into_iter()
                        .next()
                        .map(|(_, reason)| reason)
                        .or(execution_reason)
                })
                .flatten();
            let can_stop = self.nodes.iter().any(|node| {
                node.execution_id.as_deref() == Some(execution_id.as_str())
                    && node.is_leaf()
                    && node.status == WorkspaceNodeStatus::Running
                    && node.completion_signals != NodeCompletionSignalState::StopReceived
            });
            if let Some(workflow) = self.workflow_node_mut(&execution_id) {
                workflow.resume_unavailable_reason = reason;
                workflow.can_stop = can_stop;
                workflow.can_resume = (interrupted || paused)
                    && !waiting_approval
                    && workflow.resume_unavailable_reason.is_none();
            }
        }
    }

    pub(super) fn recompute_root_order(&mut self) {
        let mut direct_sessions = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.parent_id.is_none() && node.kind == WorkspaceNodeKind::Session)
            .map(|(index, node)| (index, node.title.clone(), node.id.clone()))
            .collect::<Vec<_>>();
        let mut workflows = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.parent_id.is_none() && node.kind == WorkspaceNodeKind::Workflow
            })
            .map(|(index, node)| (index, node.title.clone(), node.id.clone()))
            .collect::<Vec<_>>();
        let by_title = |left: &(usize, String, String), right: &(usize, String, String)| {
            left.1
                .to_lowercase()
                .cmp(&right.1.to_lowercase())
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        };
        direct_sessions.sort_by(by_title);
        workflows.sort_by(by_title);
        for (order, (index, _, _)) in direct_sessions.into_iter().chain(workflows).enumerate() {
            self.nodes[index].sibling_order = order as u64;
        }
    }
}

pub struct WorkspaceTreeProjector;

impl WorkspaceTreeProjector {
    pub fn session_node_id(session_id: &str) -> String {
        opaque_node_id(&format!("session\0{session_id}"))
    }

    pub fn project(
        tree: &mut WorkspaceTree,
        facts: impl IntoIterator<Item = WorkspaceStructureFact>,
    ) -> Result<bool, WorkspaceTreeError> {
        let mut applied_fact = false;
        for fact in facts {
            applied_fact = true;
            match fact {
                WorkspaceStructureFact::SessionProjected(session) => {
                    tree.apply_session(session)?;
                }
                WorkspaceStructureFact::SessionRemoved { session_id } => {
                    if let Some(node) = tree.session_node(&session_id).cloned() {
                        if node.kind == WorkspaceNodeKind::Session {
                            tree.remove_subtree(&node.id);
                        } else if let Some(bound) = tree
                            .nodes
                            .iter_mut()
                            .find(|candidate| candidate.id == node.id)
                        {
                            bound.session_id = None;
                        }
                    }
                }
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id,
                    workflow_name,
                    worktree_path,
                    definition,
                    timestamp,
                } => tree.apply_workflow_started(
                    execution_id,
                    workflow_name,
                    worktree_path,
                    definition,
                    timestamp,
                )?,
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id,
                    workflow_name,
                    status,
                    updated_at,
                } => {
                    if let Some(workflow) = tree.workflow_node_mut(&execution_id) {
                        workflow.title = non_empty_or(workflow_name, DEFAULT_WORKFLOW_TITLE);
                        workflow.status = workflow_status(status);
                        workflow.updated_at_bits = updated_at.to_bits();
                        workflow.can_stop = status.can_stop();
                        workflow.can_resume =
                            status.can_resume() && workflow.resume_unavailable_reason.is_none();
                        workflow.can_abort = status.can_abort();
                        workflow.can_archive = matches!(
                            status,
                            ExecutionStatus::Completed | ExecutionStatus::Aborted
                        );
                    }
                }
                WorkspaceStructureFact::WorkflowRemoved { execution_id } => {
                    if let Some(workflow) = tree.workflow_node(&execution_id).cloned() {
                        tree.remove_subtree(&workflow.id);
                    }
                }
                WorkspaceStructureFact::RecoveryFenceProjected { owner, reason } => {
                    if let Some(workflow) = tree.workflow_node_mut(&owner) {
                        workflow.recovery_owner_reason = reason;
                    } else if let Some(session) = tree.session_node_mut(&owner) {
                        session.recovery_owner_reason = reason;
                    }
                }
                WorkspaceStructureFact::NodeStarted {
                    execution_id,
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    fanout_parent,
                    timestamp,
                } => tree.apply_node_started(
                    execution_id,
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    fanout_parent,
                    timestamp,
                )?,
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id,
                    node_execution_id,
                    session_id,
                    timestamp,
                } => {
                    if tree
                        .nodes
                        .iter()
                        .any(|node| node.session_id.as_deref() == Some(session_id.as_str()))
                    {
                        return Err(WorkspaceTreeError::DuplicateSession(session_id));
                    }
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.session_id = Some(session_id);
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeSubmitReceived {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.completion_signals = match node.completion_signals {
                        NodeCompletionSignalState::Pending => {
                            NodeCompletionSignalState::SubmitReceived
                        }
                        NodeCompletionSignalState::StopReceived => NodeCompletionSignalState::Ready,
                        state => state,
                    };
                    node.can_retry = node.completion_signals.is_partial();
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeStopReceived {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.completion_signals = match node.completion_signals {
                        NodeCompletionSignalState::Pending => {
                            NodeCompletionSignalState::StopReceived
                        }
                        NodeCompletionSignalState::SubmitReceived => {
                            NodeCompletionSignalState::Ready
                        }
                        state => state,
                    };
                    node.can_retry = node.completion_signals.is_partial();
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodePaused {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.status = WorkspaceNodeStatus::Paused;
                    node.can_retry = node.completion_signals.is_partial();
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeResumed {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.status = WorkspaceNodeStatus::Running;
                    node.can_retry = node.completion_signals.is_partial();
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeCommandPrepared {
                    execution_id,
                    node_execution_id,
                    display_command,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.display_command = Some(display_command);
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeArtifactProduced {
                    execution_id,
                    node_execution_id,
                    result,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.has_artifact = true;
                    node.command_result = result;
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeCompleted {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => update_node_state(
                    tree,
                    &execution_id,
                    &node_execution_id,
                    WorkspaceNodeStatus::Completed,
                    None,
                    timestamp,
                )?,
                WorkspaceStructureFact::NodeFailed {
                    execution_id,
                    node_execution_id,
                    reason: _,
                    failure_kind,
                    timestamp,
                } => {
                    let (status, public_reason) =
                        if failure_kind == NodeExecutionFailureKind::UserAbort {
                            (WorkspaceNodeStatus::Aborted, "Workflow node aborted")
                        } else {
                            (WorkspaceNodeStatus::Failed, "Workflow node failed")
                        };
                    update_node_state(
                        tree,
                        &execution_id,
                        &node_execution_id,
                        status,
                        Some(public_reason.to_string()),
                        timestamp,
                    )?;
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .expect("failed Node was resolved before state update");
                    node.can_retry = failure_kind != NodeExecutionFailureKind::UserAbort;
                }
                WorkspaceStructureFact::NodeApprovalRequested {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.status = WorkspaceNodeStatus::Waiting;
                    node.can_approve = true;
                    node.can_retry = false;
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeApprovalResolved {
                    execution_id,
                    node_execution_id,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.status = WorkspaceNodeStatus::Running;
                    node.can_approve = false;
                    node.can_retry = node.completion_signals.is_partial();
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
            }
        }
        tree.recompute_root_order();
        tree.recompute_branch_summaries();
        tree.validate()?;
        if !applied_fact {
            return Ok(false);
        }
        Ok(true)
    }
}

fn update_node_state(
    tree: &mut WorkspaceTree,
    execution_id: &str,
    node_execution_id: &str,
    status: WorkspaceNodeStatus,
    error_reason: Option<String>,
    timestamp: f64,
) -> Result<(), WorkspaceTreeError> {
    let node = tree
        .execution_node_mut(execution_id, node_execution_id)
        .ok_or_else(|| WorkspaceTreeError::MissingNodeExecution(node_execution_id.to_string()))?;
    node.status = status;
    node.error_reason = error_reason;
    node.can_approve = false;
    node.can_retry = false;
    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
    Ok(())
}

fn node_shape_is_valid(node: &WorkspaceTreeNode) -> bool {
    let has_execution = node
        .execution_id
        .as_deref()
        .is_some_and(|id| !id.is_empty());
    let has_node_execution = node
        .node_execution_id
        .as_deref()
        .is_some_and(|id| !id.is_empty());
    let has_node_name = node
        .node_name
        .as_deref()
        .is_some_and(|name| !name.is_empty());
    match node.kind {
        WorkspaceNodeKind::Session => {
            node.parent_id.is_none()
                && node.session_id.as_deref().is_some_and(|id| !id.is_empty())
                && !has_execution
                && !has_node_execution
                && !has_node_name
                && node.attempt.is_none()
                && node.recovery_owner_reason.is_none()
                && node.display_command.is_none()
                && node.command_result.is_none()
        }
        WorkspaceNodeKind::Workflow => {
            node.parent_id.is_none()
                && has_execution
                && node.node_execution_id.is_none()
                && node.node_name.is_none()
                && node.attempt.is_none()
                && node.session_id.is_none()
                && node.display_command.is_none()
                && node.command_result.is_none()
        }
        WorkspaceNodeKind::Fanout => {
            let structural_shape = if node.is_internal_rule_record() {
                node.parent_id.is_some()
                    && !has_node_execution
                    && !has_node_name
                    && node.attempt.is_none()
                    && node.dynamic_fanout
            } else {
                node.parent_id.is_some()
                    && has_node_execution
                    && has_node_name
                    && node.attempt.is_some()
            };
            has_execution
                && structural_shape
                && node.session_id.is_none()
                && node.recovery_owner_reason.is_none()
                && node.display_command.is_none()
                && node.command_result.is_none()
        }
        WorkspaceNodeKind::WorkflowSession => {
            node.parent_id.is_some()
                && has_execution
                && has_node_execution
                && has_node_name
                && node.attempt.is_some()
                && node.display_command.is_none()
                && node.command_result.is_none()
        }
        WorkspaceNodeKind::WorkflowCommand => {
            node.parent_id.is_some()
                && has_execution
                && has_node_execution
                && has_node_name
                && node.attempt.is_some()
                && node.session_id.is_none()
                && node.recovery_owner_reason.is_none()
        }
    }
}

fn session_status(state: WorkspaceSessionState) -> WorkspaceNodeStatus {
    match state {
        WorkspaceSessionState::Active => WorkspaceNodeStatus::Running,
        WorkspaceSessionState::Idle => WorkspaceNodeStatus::Waiting,
        WorkspaceSessionState::Done
        | WorkspaceSessionState::Closed
        | WorkspaceSessionState::Archived => WorkspaceNodeStatus::Completed,
        WorkspaceSessionState::Error => WorkspaceNodeStatus::Error,
    }
}

pub(super) fn workflow_status(status: ExecutionStatus) -> WorkspaceNodeStatus {
    match status {
        ExecutionStatus::Running => WorkspaceNodeStatus::Running,
        #[cfg(test)]
        ExecutionStatus::WaitingApproval => WorkspaceNodeStatus::Waiting,
        #[cfg(test)]
        ExecutionStatus::Interrupted => WorkspaceNodeStatus::Interrupted,
        ExecutionStatus::Completed => WorkspaceNodeStatus::Completed,
        ExecutionStatus::Aborted => WorkspaceNodeStatus::Aborted,
    }
}

fn aggregate_status(
    children: &[WorkspaceNodeStatus],
    parent: WorkspaceNodeStatus,
) -> WorkspaceNodeStatus {
    children
        .iter()
        .copied()
        .chain(std::iter::once(parent))
        .min_by_key(|status| match status {
            WorkspaceNodeStatus::Running => 1,
            WorkspaceNodeStatus::Paused => 2,
            WorkspaceNodeStatus::Failed => 3,
            WorkspaceNodeStatus::Error => 4,
            WorkspaceNodeStatus::Waiting => 5,
            WorkspaceNodeStatus::Interrupted => 6,
            WorkspaceNodeStatus::Aborted => 7,
            WorkspaceNodeStatus::Completed => 8,
        })
        .unwrap_or(parent)
}

pub(super) fn non_empty_or(value: String, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn max_f64_bits(left: u64, right: u64) -> u64 {
    if f64::from_bits(left)
        .total_cmp(&f64::from_bits(right))
        .is_lt()
    {
        right
    } else {
        left
    }
}

fn workflow_node_occurrence_key(node_name: &str, occurrence: usize) -> String {
    if occurrence == 0 {
        format!("node\0{node_name}")
    } else {
        format!("node\0{node_name}\0occurrence\0{occurrence}")
    }
}

fn fanout_branch_occurrence_key(
    execution_id: &str,
    fanout_name: &str,
    occurrence: usize,
) -> String {
    if occurrence == 0 {
        format!("workflow\0{execution_id}\0fanout\0{fanout_name}")
    } else {
        format!("workflow\0{execution_id}\0fanout\0{fanout_name}\0occurrence\0{occurrence}")
    }
}

fn fanout_child_occurrence_key(
    fanout_name: &str,
    parent_occurrence: usize,
    item_index: Option<usize>,
    child_index: usize,
    child_name: &str,
    occurrence: usize,
) -> String {
    if parent_occurrence == 0 && occurrence == 0 {
        format!("fanout-child\0{fanout_name}\0{item_index:?}\0{child_index}\0{child_name}")
    } else {
        format!(
            "fanout-child\0{fanout_name}\0parent-occurrence\0{parent_occurrence}\0{item_index:?}\0{child_index}\0{child_name}\0occurrence\0{occurrence}"
        )
    }
}

fn fanout_dynamic_child_occurrence_key(
    fanout_name: &str,
    parent_occurrence: usize,
    child_index: usize,
    child_name: &str,
    occurrence: usize,
) -> String {
    fanout_child_occurrence_key(
        fanout_name,
        parent_occurrence,
        None,
        child_index,
        child_name,
        occurrence,
    )
}

fn workflow_occurrence(nodes: &[WorkspaceTreeNode], fanout_id: &str) -> usize {
    let Some(fanout) = nodes.iter().find(|node| node.id == fanout_id) else {
        return 0;
    };
    nodes
        .iter()
        .filter(|node| {
            node.parent_id == fanout.parent_id
                && node.node_name == fanout.node_name
                && node.sibling_order < fanout.sibling_order
        })
        .count()
}

fn opaque_node_id(semantic_key: &str) -> String {
    opaque_id("node", semantic_key)
}

fn opaque_workflow_node_id(
    execution_id: &str,
    semantic_key: &str,
) -> Result<String, WorkspaceTreeError> {
    let execution_id =
        normalized_uuid_simple(execution_id).ok_or(WorkspaceTreeError::IdentityMismatch)?;
    let digest = Sha256::digest(semantic_key.as_bytes());
    Ok(format!(
        "node-w-{execution_id}-{}",
        hex::encode(&digest[..16])
    ))
}

fn opaque_branch_id(semantic_key: &str) -> String {
    opaque_id("branch", semantic_key)
}

fn opaque_id(prefix: &str, semantic_key: &str) -> String {
    let digest = Sha256::digest(semantic_key.as_bytes());
    format!("{prefix}-{}", hex::encode(&digest[..16]))
}

fn normalized_uuid_simple(value: &str) -> Option<String> {
    let value = value
        .strip_prefix("urn:uuid:")
        .or_else(|| value.strip_prefix("URN:UUID:"))
        .unwrap_or(value);
    let value = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(value);
    let mut normalized = String::with_capacity(32);
    for character in value.chars() {
        if character == '-' {
            continue;
        }
        if !character.is_ascii_hexdigit() || normalized.len() == 32 {
            return None;
        }
        normalized.push(character.to_ascii_lowercase());
    }
    (normalized.len() == 32).then_some(normalized)
}

fn dynamic_fanout_sentinel_id(execution_id: &str, node_name: &str) -> String {
    opaque_id(
        "workspace-internal-dynamic-fanout",
        &format!("{execution_id}\0{node_name}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workflow::{
        FacetRefs, FanoutSpec, ItemsSource, NodeDefinition, NodeKind, SessionGate, SessionSpec,
    };
    use crate::domain::workspace_tree::WorkspaceTreeVisibilityPolicy;

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            name: "review".to_string(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::new(),
            nodes: vec![NodeDefinition {
                name: "plan".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: ProviderKind::Claude,
                    gate: SessionGate::Auto,
                    facets: FacetRefs::default(),
                }),
                artifact: None,
                input: None,
                inputs: Vec::new(),
                rules: Vec::new(),
            }],
        }
    }

    #[test]
    fn workspace_tree_projector_owns_parentage_identity_and_occurrence_order() {
        let execution_id = "00000000-0000-4000-8000-000000000001";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 0,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-2".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 3.0,
                },
            ],
        )
        .unwrap();

        let leaves = tree
            .nodes()
            .iter()
            .filter(|node| node.kind == WorkspaceNodeKind::WorkflowSession)
            .collect::<Vec<_>>();
        assert_eq!(leaves.len(), 2);
        assert_ne!(leaves[0].id, leaves[1].id);
        assert_eq!(leaves[0].parent_id.as_deref(), Some(execution_id));
        assert!(leaves[0].sibling_order < leaves[1].sibling_order);
    }

    #[test]
    fn workflow_without_started_nodes_has_an_empty_branch_and_no_preferred_node() {
        let execution_id = "00000000-0000-4000-8000-000000000099";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            }],
        )
        .unwrap();

        let public_nodes = tree
            .nodes()
            .iter()
            .filter(|node| !node.is_internal_rule_record())
            .collect::<Vec<_>>();
        assert_eq!(public_nodes.len(), 1);
        assert_eq!(public_nodes[0].kind, WorkspaceNodeKind::Workflow);
        assert_eq!(tree.preferred_node_id(&HashSet::new()), None);
    }

    #[test]
    fn workspace_tree_rejects_duplicate_session_binding() {
        let node = WorkspaceTreeNode {
            id: "a".to_string(),
            parent_id: None,
            sibling_order: 0,
            kind: WorkspaceNodeKind::Session,
            title: "a".to_string(),
            status: WorkspaceNodeStatus::Waiting,
            error_reason: None,
            updated_at_bits: 0.0f64.to_bits(),
            execution_id: None,
            node_execution_id: None,
            node_name: None,
            attempt: None,
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: Some("session".to_string()),
            can_approve: false,
            can_retry: false,
            can_close: true,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        };
        let mut duplicate = node.clone();
        duplicate.id = "b".to_string();
        assert!(matches!(
            WorkspaceTree::restore("/repo", vec![node, duplicate]),
            Err(WorkspaceTreeError::DuplicateSession(_))
        ));
    }

    fn two_execution_session_tree() -> WorkspaceTree {
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: "00000000-0000-4000-8000-0000000000a1".to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: "00000000-0000-4000-8000-0000000000a1".to_string(),
                    node_execution_id: "node-a".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: "00000000-0000-4000-8000-0000000000b1".to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: "00000000-0000-4000-8000-0000000000b1".to_string(),
                    node_execution_id: "node-b".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 4.0,
                },
            ],
        )
        .unwrap();
        tree
    }

    fn workflow_session_fact(
        session_id: &str,
        execution_id: &str,
        node_execution_id: &str,
        state: WorkspaceSessionState,
        updated_at: f64,
    ) -> WorkspaceStructureFact {
        WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
            id: session_id.to_string(),
            worktree_path: "/repo".to_string(),
            state,
            error_reason: (state == WorkspaceSessionState::Error)
                .then(|| "session failed".to_string()),
            updated_at_bits: updated_at.to_bits(),
            title: Some("Plan".to_string()),
            first_message: String::new(),
            workflow_node_session: true,
            workflow_execution_id: Some(execution_id.to_string()),
            workflow_node_execution_id: Some(node_execution_id.to_string()),
            unresolved_recovery_reason: None,
        })
    }

    #[test]
    fn workflow_session_fact_does_not_bind_an_execution_and_node_from_different_runs() {
        let mut tree = two_execution_session_tree();

        WorkspaceTreeProjector::project(
            &mut tree,
            [workflow_session_fact(
                "crossed-session",
                "00000000-0000-4000-8000-0000000000a1",
                "node-b",
                WorkspaceSessionState::Active,
                5.0,
            )],
        )
        .unwrap();

        assert!(tree
            .nodes()
            .iter()
            .all(|node| node.session_id.as_deref() != Some("crossed-session")));
    }

    #[test]
    fn workflow_session_fact_binds_a_matching_execution_and_node_pair() {
        let mut tree = two_execution_session_tree();

        WorkspaceTreeProjector::project(
            &mut tree,
            [workflow_session_fact(
                "matched-session",
                "00000000-0000-4000-8000-0000000000b1",
                "node-b",
                WorkspaceSessionState::Active,
                5.0,
            )],
        )
        .unwrap();

        let node = tree.session_node("matched-session").unwrap();
        assert_eq!(
            node.execution_id.as_deref(),
            Some("00000000-0000-4000-8000-0000000000b1")
        );
        assert_eq!(node.node_execution_id.as_deref(), Some("node-b"));
    }

    #[test]
    fn workflow_session_fact_rebinds_without_overwriting_node_lifecycle_state() {
        let mut tree = two_execution_session_tree();
        WorkspaceTreeProjector::project(
            &mut tree,
            [workflow_session_fact(
                "rebound-session",
                "00000000-0000-4000-8000-0000000000a1",
                "node-a",
                WorkspaceSessionState::Active,
                5.0,
            )],
        )
        .unwrap();

        WorkspaceTreeProjector::project(
            &mut tree,
            [workflow_session_fact(
                "rebound-session",
                "unrelated-execution",
                "unrelated-node",
                WorkspaceSessionState::Error,
                6.0,
            )],
        )
        .unwrap();

        let node = tree.session_node("rebound-session").unwrap();
        assert_eq!(
            node.execution_id.as_deref(),
            Some("00000000-0000-4000-8000-0000000000a1")
        );
        assert_eq!(node.node_execution_id.as_deref(), Some("node-a"));
        assert_eq!(node.status, WorkspaceNodeStatus::Running);
        assert_eq!(node.error_reason, None);
        assert_eq!(node.updated_at_bits, 6.0f64.to_bits());
    }

    #[test]
    fn closed_direct_session_leaves_tree_but_closed_workflow_session_keeps_structure() {
        let direct_fact = |state| WorkspaceSessionFact {
            id: "direct-session".to_string(),
            worktree_path: "/repo".to_string(),
            state,
            error_reason: None,
            updated_at_bits: 4.0f64.to_bits(),
            title: Some("Direct".to_string()),
            first_message: "Direct".to_string(),
            workflow_node_session: false,
            workflow_execution_id: None,
            workflow_node_execution_id: None,
            unresolved_recovery_reason: None,
        };
        let mut direct_tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut direct_tree,
            [WorkspaceStructureFact::SessionProjected(direct_fact(
                WorkspaceSessionState::Idle,
            ))],
        )
        .unwrap();
        assert_eq!(direct_tree.nodes().len(), 1);

        WorkspaceTreeProjector::project(
            &mut direct_tree,
            [WorkspaceStructureFact::SessionProjected(direct_fact(
                WorkspaceSessionState::Closed,
            ))],
        )
        .unwrap();
        assert!(direct_tree.nodes().is_empty());

        let execution_id = "00000000-0000-4000-8000-000000000001";
        let mut workflow_tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut workflow_tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "node-execution".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 0,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "node-execution".to_string(),
                    session_id: "workflow-session".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
                    id: "workflow-session".to_string(),
                    worktree_path: "/repo".to_string(),
                    state: WorkspaceSessionState::Closed,
                    error_reason: None,
                    updated_at_bits: 4.0f64.to_bits(),
                    title: Some("Plan".to_string()),
                    first_message: "Plan".to_string(),
                    workflow_node_session: true,
                    workflow_execution_id: Some(execution_id.to_string()),
                    workflow_node_execution_id: Some("node-execution".to_string()),
                    unresolved_recovery_reason: None,
                }),
            ],
        )
        .unwrap();
        assert!(workflow_tree
            .session_node("workflow-session")
            .is_some_and(|node| node.kind == WorkspaceNodeKind::WorkflowSession));
    }

    #[test]
    fn archive_visibility_hides_only_workflow_branch() {
        let execution_id = "00000000-0000-4000-8000-000000000001";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [WorkspaceStructureFact::WorkflowStarted {
                execution_id: execution_id.to_string(),
                workflow_name: "review".to_string(),
                worktree_path: "/repo".to_string(),
                definition: definition(),
                timestamp: 1.0,
            }],
        )
        .unwrap();
        let hidden = WorkspaceTreeVisibilityPolicy::hidden_branch_ids(&tree, [execution_id]);
        assert_eq!(hidden, HashSet::from([execution_id.to_string()]));
        assert_eq!(tree.nodes().len(), 1);
    }

    #[test]
    fn workflow_recovery_reason_is_derived_from_stable_owner_order() {
        let execution_id = "00000000-0000-4000-8000-000000000001";
        let session_fact = |session_id: &str, node_execution_id: &str, reason: Option<&str>| {
            WorkspaceSessionFact {
                id: session_id.to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Idle,
                error_reason: None,
                updated_at_bits: 4.0f64.to_bits(),
                title: None,
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(execution_id.to_string()),
                workflow_node_execution_id: Some(node_execution_id.to_string()),
                unresolved_recovery_reason: reason.map(str::to_string),
            }
        };
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-z".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 0,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-z".to_string(),
                    session_id: "session-z".to_string(),
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-a".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "n-a".to_string(),
                    session_id: "session-a".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::SessionProjected(session_fact(
                    "session-z",
                    "n-z",
                    Some("session-z recovery"),
                )),
                WorkspaceStructureFact::SessionProjected(session_fact(
                    "session-a",
                    "n-a",
                    Some("session-a recovery"),
                )),
                WorkspaceStructureFact::RecoveryFenceProjected {
                    owner: execution_id.to_string(),
                    reason: Some("execution recovery".to_string()),
                },
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    status: ExecutionStatus::Interrupted,
                    updated_at: 5.0,
                },
            ],
        )
        .unwrap();

        let workflow = tree.workflow_node(execution_id).unwrap();
        assert_eq!(
            workflow.resume_unavailable_reason.as_deref(),
            Some("session-a recovery")
        );
        assert!(!workflow.can_resume);

        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::RecoveryFenceProjected {
                    owner: "session-a".to_string(),
                    reason: None,
                },
                WorkspaceStructureFact::RecoveryFenceProjected {
                    owner: execution_id.to_string(),
                    reason: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            tree.workflow_node(execution_id)
                .unwrap()
                .resume_unavailable_reason
                .as_deref(),
            Some("session-z recovery")
        );
    }

    #[test]
    fn opaque_identity_digest_matches_sha256_and_normalizes_uuid() {
        assert_eq!(
            opaque_id("node", "abc"),
            "node-ba7816bf8f01cfea414140de5dae2223"
        );
        assert_eq!(
            opaque_workflow_node_id("{00000000-0000-4000-8000-000000000001}", "abc").unwrap(),
            "node-w-00000000000040008000000000000001-ba7816bf8f01cfea414140de5dae2223"
        );
    }

    #[test]
    fn literal_fanout_projects_only_started_children_in_event_order() {
        let execution_id = "00000000-0000-4000-8000-000000000149";
        let mut definition = definition();
        definition.nodes.push(NodeDefinition {
            name: "fanout".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["child-a".to_string(), "child-b".to_string()],
                items: Some(ItemsSource::Literal(vec![serde_json::json!("only")])),
            }),
            ..NodeDefinition::default()
        });
        let mut tree = WorkspaceTree::empty("/repo");

        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition,
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-execution".to_string(),
                    node_name: "fanout".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "child-b-execution".to_string(),
                    node_name: "child-b".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "fanout".to_string(),
                        parent_attempt: 1,
                        item_index: Some(0),
                        child_index: 1,
                    }),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "child-a-execution".to_string(),
                    node_name: "child-a".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "fanout".to_string(),
                        parent_attempt: 1,
                        item_index: Some(0),
                        child_index: 0,
                    }),
                    timestamp: 4.0,
                },
            ],
        )
        .unwrap();

        let fanout = tree
            .nodes()
            .iter()
            .find(|node| node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record())
            .unwrap();
        let mut children = tree
            .nodes()
            .iter()
            .filter(|node| node.parent_id.as_deref() == Some(fanout.id.as_str()))
            .collect::<Vec<_>>();
        children.sort_by_key(|node| node.sibling_order);
        assert_eq!(
            children
                .iter()
                .map(|node| node.title.as_str())
                .collect::<Vec<_>>(),
            vec!["child-b", "child-a"]
        );
    }

    #[test]
    fn artifact_item_fanout_without_started_children_has_an_empty_branch() {
        let execution_id = "00000000-0000-4000-8000-000000000151";
        let mut workflow = definition();
        workflow.nodes.push(NodeDefinition {
            name: "matrix".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["plan".to_string()],
                items: Some(ItemsSource::ArtifactField {
                    node: "source".to_string(),
                    field: "items".to_string(),
                }),
            }),
            ..NodeDefinition::default()
        });
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: workflow,
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "matrix-parent".to_string(),
                    node_name: "matrix".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
            ],
        )
        .unwrap();

        let fanout = tree
            .nodes()
            .iter()
            .find(|node| node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record())
            .unwrap();
        let fanout_id = fanout.id.clone();
        assert!(!tree
            .nodes()
            .iter()
            .any(|node| node.parent_id.as_deref() == Some(fanout_id.as_str())));

        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "matrix-child-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "matrix".to_string(),
                        parent_attempt: 1,
                        item_index: Some(0),
                        child_index: 0,
                    }),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "matrix-child-2".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "matrix".to_string(),
                        parent_attempt: 1,
                        item_index: Some(1),
                        child_index: 0,
                    }),
                    timestamp: 4.0,
                },
            ],
        )
        .unwrap();
        let children = tree
            .nodes()
            .iter()
            .filter(|node| node.parent_id.as_deref() == Some(fanout_id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_ne!(children[0].id, children[1].id);
    }

    #[test]
    fn fanout_occurrences_are_distinct_and_children_stay_nested_in_event_order() {
        let execution_id = "00000000-0000-4000-8000-000000000152";
        let mut workflow = definition();
        workflow.nodes.push(NodeDefinition {
            name: "reviews".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["plan".to_string()],
                items: None,
            }),
            ..NodeDefinition::default()
        });
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: workflow,
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-1".to_string(),
                    node_name: "reviews".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-1-child-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "reviews".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 0,
                    }),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-1-child-2".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 2,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "reviews".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 0,
                    }),
                    timestamp: 4.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-2".to_string(),
                    node_name: "reviews".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 2,
                    fanout_parent: None,
                    timestamp: 5.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "fanout-2-child".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 3,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "reviews".to_string(),
                        parent_attempt: 2,
                        item_index: None,
                        child_index: 0,
                    }),
                    timestamp: 6.0,
                },
            ],
        )
        .unwrap();

        let mut fanouts = tree
            .nodes()
            .iter()
            .filter(|node| {
                node.kind == WorkspaceNodeKind::Fanout && !node.is_internal_rule_record()
            })
            .collect::<Vec<_>>();
        fanouts.sort_by_key(|node| node.sibling_order);
        assert_eq!(fanouts.len(), 2);
        assert_ne!(fanouts[0].id, fanouts[1].id);
        let children = |parent_id: &str| {
            tree.nodes()
                .iter()
                .filter(|node| node.parent_id.as_deref() == Some(parent_id))
                .collect::<Vec<_>>()
        };
        let first_children = children(&fanouts[0].id);
        let second_children = children(&fanouts[1].id);
        assert_eq!(first_children.len(), 2);
        assert_eq!(second_children.len(), 1);
        assert_ne!(first_children[0].id, first_children[1].id);
    }

    #[test]
    fn branch_status_capabilities_and_session_activity_are_backend_aggregated() {
        let execution_id = "00000000-0000-4000-8000-000000000153";
        let mut workflow = definition();
        workflow.nodes.extend([
            NodeDefinition {
                name: "checks".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    child: vec!["lint".to_string(), "test".to_string()],
                    items: None,
                }),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "lint".to_string(),
                ..NodeDefinition::default()
            },
            NodeDefinition {
                name: "test".to_string(),
                ..NodeDefinition::default()
            },
        ]);
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: workflow,
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "plan".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "plan".to_string(),
                    session_id: "plan-session".to_string(),
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "plan".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "checks".to_string(),
                    node_name: "checks".to_string(),
                    kind: NodeKindName::Fanout,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 4.0,
                },
                WorkspaceStructureFact::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "checks".to_string(),
                    timestamp: 5.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "lint".to_string(),
                    node_name: "lint".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "checks".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 0,
                    }),
                    timestamp: 6.0,
                },
                WorkspaceStructureFact::NodeFailed {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "lint".to_string(),
                    reason: "internal lint failure".to_string(),
                    failure_kind: NodeExecutionFailureKind::ValidationFailure,
                    timestamp: 7.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "test".to_string(),
                    node_name: "test".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    fanout_parent: Some(FanoutParentRef {
                        parent_node: "checks".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index: 1,
                    }),
                    timestamp: 8.0,
                },
                WorkspaceStructureFact::NodeApprovalRequested {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "test".to_string(),
                    timestamp: 9.0,
                },
                WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
                    id: "plan-session".to_string(),
                    worktree_path: "/repo".to_string(),
                    state: WorkspaceSessionState::Active,
                    error_reason: None,
                    updated_at_bits: 10.0f64.to_bits(),
                    title: Some("Plan".to_string()),
                    first_message: String::new(),
                    workflow_node_session: true,
                    workflow_execution_id: Some(execution_id.to_string()),
                    workflow_node_execution_id: Some("plan".to_string()),
                    unresolved_recovery_reason: None,
                }),
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    status: ExecutionStatus::WaitingApproval,
                    updated_at: 10.0,
                },
            ],
        )
        .unwrap();

        let workflow = tree.workflow_node(execution_id).unwrap();
        assert_eq!(workflow.status, WorkspaceNodeStatus::Failed);
        assert!(!workflow.can_stop);
        assert!(!workflow.can_resume);
        assert!(workflow.can_abort);
        assert!(!workflow.can_archive);
        assert_eq!(
            tree.session_node("plan-session").unwrap().status,
            WorkspaceNodeStatus::Completed
        );
        let fanout = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("checks"))
            .unwrap();
        assert_eq!(fanout.status, WorkspaceNodeStatus::Failed);
        let waiting = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("test"))
            .unwrap();
        assert_eq!(waiting.status, WorkspaceNodeStatus::Waiting);
        assert!(waiting.can_approve);
    }

    #[test]
    fn active_workflow_root_display_is_derived_from_the_current_node() {
        let execution_id = "00000000-0000-4000-8000-000000000154";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "plan".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodePaused {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "plan".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    status: ExecutionStatus::Running,
                    updated_at: 3.0,
                },
            ],
        )
        .unwrap();

        let workflow = tree.workflow_node(execution_id).unwrap();
        assert_eq!(workflow.status, WorkspaceNodeStatus::Paused);
        assert!(!workflow.can_stop);
        assert!(workflow.can_resume);
    }

    #[test]
    fn terminal_workflows_hide_every_unstarted_leaf_and_branch() {
        let execution_id = "00000000-0000-4000-8000-000000000150";
        let mut workflow = definition();
        workflow.nodes.push(NodeDefinition {
            name: "dynamic".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["plan".to_string()],
                items: Some(ItemsSource::ArtifactField {
                    node: "source".to_string(),
                    field: "items".to_string(),
                }),
            }),
            ..NodeDefinition::default()
        });
        let mut tree = WorkspaceTree::empty("/repo");

        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: workflow,
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    status: ExecutionStatus::Completed,
                    updated_at: 2.0,
                },
            ],
        )
        .unwrap();

        let public_nodes = tree
            .nodes()
            .iter()
            .filter(|node| !node.is_internal_rule_record())
            .collect::<Vec<_>>();
        assert_eq!(public_nodes.len(), 1);
        assert_eq!(public_nodes[0].kind, WorkspaceNodeKind::Workflow);
        assert_eq!(public_nodes[0].status, WorkspaceNodeStatus::Completed);
    }

    #[test]
    fn root_order_and_opaque_ids_match_the_audit_golden() {
        let direct = |id: &str, title: &str, updated_at: f64| {
            WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
                id: id.to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Idle,
                error_reason: None,
                updated_at_bits: updated_at.to_bits(),
                title: Some(title.to_string()),
                first_message: title.to_string(),
                workflow_node_session: false,
                workflow_execution_id: None,
                workflow_node_execution_id: None,
                unresolved_recovery_reason: None,
            })
        };
        let mut tree = WorkspaceTree::empty("/repo");

        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: "00000000-0000-4000-8000-000000000002".to_string(),
                    workflow_name: "zeta".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                direct("session-beta", "beta", 2.0),
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
                    workflow_name: "Echo".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 3.0,
                },
                direct("session-alpha", "Alpha", 4.0),
            ],
        )
        .unwrap();

        let mut roots = tree
            .nodes()
            .iter()
            .filter(|node| node.parent_id.is_none())
            .collect::<Vec<_>>();
        roots.sort_by_key(|node| node.sibling_order);
        assert_eq!(
            roots
                .iter()
                .map(|node| (node.id.as_str(), node.title.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("node-25e8040f94a3a76d0f57303c2ed95343", "Alpha",),
                ("node-6383f1571670eb90ad7bdde81c9c4126", "beta",),
                ("00000000-0000-4000-8000-000000000001", "Echo"),
                ("00000000-0000-4000-8000-000000000002", "zeta"),
            ]
        );
    }

    fn repeated_command_occurrence_tree() -> WorkspaceTree {
        let execution_id = "00000000-0000-4000-8000-000000000011";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeCommandPrepared {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-1".to_string(),
                    display_command: "first command".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-1".to_string(),
                    timestamp: 4.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-2".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Command,
                    attempt: 2,
                    fanout_parent: None,
                    timestamp: 5.0,
                },
                WorkspaceStructureFact::NodeCommandPrepared {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-2".to_string(),
                    display_command: "second command".to_string(),
                    timestamp: 6.0,
                },
                WorkspaceStructureFact::NodeApprovalRequested {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "occurrence-2".to_string(),
                    timestamp: 7.0,
                },
            ],
        )
        .unwrap();
        tree
    }

    #[test]
    fn each_occurrence_keeps_its_detail_and_only_waiting_occurrence_can_approve() {
        let tree = repeated_command_occurrence_tree();
        let first = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("occurrence-1"))
            .unwrap();
        let second = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("occurrence-2"))
            .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.display_command.as_deref(), Some("first command"));
        assert!(!first.can_approve);
        assert_eq!(second.display_command.as_deref(), Some("second command"));
        assert!(second.can_approve);
    }

    #[test]
    fn command_detail_remains_bound_to_the_selected_occurrence() {
        let tree = repeated_command_occurrence_tree();
        let first = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("occurrence-1"))
            .unwrap();
        let second = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("occurrence-2"))
            .unwrap();

        assert_eq!(first.display_command.as_deref(), Some("first command"));
        assert_eq!(second.display_command.as_deref(), Some("second command"));
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn repeated_session_occurrences_keep_distinct_session_detail_and_lookup() {
        let execution_id = "00000000-0000-4000-8000-000000000013";
        let session_fact = |id: &str, node_execution_id: &str, state| {
            WorkspaceStructureFact::SessionProjected(WorkspaceSessionFact {
                id: id.to_string(),
                worktree_path: "/repo".to_string(),
                state,
                error_reason: None,
                updated_at_bits: 5.0f64.to_bits(),
                title: Some("Review".to_string()),
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(execution_id.to_string()),
                workflow_node_execution_id: Some(node_execution_id.to_string()),
                unresolved_recovery_reason: None,
            })
        };
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "session-occurrence-1".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "session-occurrence-1".to_string(),
                    session_id: "stored-session-1".to_string(),
                    timestamp: 2.0,
                },
                WorkspaceStructureFact::NodeCompleted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "session-occurrence-1".to_string(),
                    timestamp: 3.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "session-occurrence-2".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 2,
                    fanout_parent: None,
                    timestamp: 4.0,
                },
                WorkspaceStructureFact::NodeAgentBound {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "session-occurrence-2".to_string(),
                    session_id: "stored-session-2".to_string(),
                    timestamp: 4.0,
                },
                session_fact(
                    "stored-session-1",
                    "session-occurrence-1",
                    WorkspaceSessionState::Done,
                ),
                session_fact(
                    "stored-session-2",
                    "session-occurrence-2",
                    WorkspaceSessionState::Active,
                ),
            ],
        )
        .unwrap();

        let first = tree.session_node("stored-session-1").unwrap();
        let second = tree.session_node("stored-session-2").unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.session_id.as_deref(), Some("stored-session-1"));
        assert_eq!(second.session_id.as_deref(), Some("stored-session-2"));
        assert_eq!(first.status, WorkspaceNodeStatus::Completed);
        assert_eq!(second.status, WorkspaceNodeStatus::Running);
    }

    #[test]
    fn missing_session_keeps_node_but_returns_no_unusable_session_id() {
        let execution_id = "00000000-0000-4000-8000-000000000012";
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: "review".to_string(),
                    worktree_path: "/repo".to_string(),
                    definition: definition(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::NodeStarted {
                    execution_id: execution_id.to_string(),
                    node_execution_id: "missing-session".to_string(),
                    node_name: "plan".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: 2.0,
                },
            ],
        )
        .unwrap();
        let node = tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some("missing-session"))
            .unwrap();
        assert_eq!(node.session_id, None);
        assert!(!node.can_close);
    }

    #[test]
    fn direct_session_error_reason_is_preserved_for_badge_and_detail_reads() {
        let mut tree = WorkspaceTree::empty("/repo");
        WorkspaceTreeProjector::project(
            &mut tree,
            [WorkspaceStructureFact::SessionProjected(
                WorkspaceSessionFact {
                    id: "errored-session".to_string(),
                    worktree_path: "/repo".to_string(),
                    state: WorkspaceSessionState::Error,
                    error_reason: Some("provider failed".to_string()),
                    updated_at_bits: 2.0f64.to_bits(),
                    title: Some("Errored".to_string()),
                    first_message: "Errored".to_string(),
                    workflow_node_session: false,
                    workflow_execution_id: None,
                    workflow_node_execution_id: None,
                    unresolved_recovery_reason: None,
                },
            )],
        )
        .unwrap();
        let node = tree.session_node("errored-session").unwrap();
        assert_eq!(node.status, WorkspaceNodeStatus::Error);
        assert_eq!(node.error_reason.as_deref(), Some("provider failed"));
    }
}
