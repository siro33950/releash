//! Durable Workspace tree aggregate and projector.
//!
//! This aggregate owns structural participation, parentage, sibling order,
//! opaque identities, and the one-to-one Node/Session bindings. Persistence
//! records and public DTOs deliberately live outside this module.

use std::collections::{BTreeMap, HashSet};

use sha2::{Digest, Sha256};

use super::value_objects::{
    WorkspaceExecution, WorkspaceIdentity, WorkspaceNodeKind, WorkspaceNodeStatus,
    WorkspaceStructureFact, WorkspaceTreeError, WorkspaceTreeNode, INTERNAL_SIBLING_ORDER,
};
use super::{WorkspaceNodeStatusClassification, WorkspacePublicRoot};
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionParentRef, ExecutionStatus, NodeCompletionSignalState,
    NodeExecutionFailureKind, NodeKindName,
};

pub(super) const DEFAULT_WORKFLOW_TITLE: &str = "Workflow";

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceTree {
    workspace_identity: WorkspaceIdentity,
    nodes: Vec<WorkspaceTreeNode>,
    executions: Vec<WorkspaceExecution>,
}

impl WorkspaceTree {
    /// この Workspace に属する実行木ごとの状態を記録する。
    pub(crate) fn record_executions(&mut self, executions: Vec<WorkspaceExecution>) {
        self.executions = executions;
    }

    pub fn executions(&self) -> &[WorkspaceExecution] {
        &self.executions
    }

    pub fn workspace_identity(&self) -> &WorkspaceIdentity {
        &self.workspace_identity
    }

    pub fn observe_background_failure(&mut self, target: &str, message: &str) {
        observe_background_failures(&mut self.nodes, [(target, message)]);
    }

    pub fn empty(workspace_identity: impl Into<String>) -> Self {
        Self {
            workspace_identity: WorkspaceIdentity::new(workspace_identity.into()),
            nodes: Vec::new(),
            executions: Vec::new(),
        }
    }

    pub fn restore(
        workspace_identity: impl Into<String>,
        nodes: Vec<WorkspaceTreeNode>,
    ) -> Result<Self, WorkspaceTreeError> {
        let mut tree = Self {
            workspace_identity: WorkspaceIdentity::new(workspace_identity.into()),
            nodes,
            executions: Vec::new(),
        };
        tree.recompute_root_order();
        tree.recompute_retry_histories();
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
            if !node.is_internal_rule_record() && !node.is_retry_history {
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
                collect(Some(&node.id), children, hidden, leaves);
                if node.is_leaf() {
                    leaves.push(node);
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn session_node(&self, session_id: &str) -> Option<&WorkspaceTreeNode> {
        self.nodes
            .iter()
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
                    WorkspaceNodeKind::Workflow
                        | WorkspaceNodeKind::Fanout
                        | WorkspaceNodeKind::Sequence
                        | WorkspaceNodeKind::WorkflowSession
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

    pub(super) fn apply_workflow_started(
        &mut self,
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        dynamic_fanout_names: std::collections::BTreeSet<String>,
        timestamp: f64,
    ) -> Result<(), WorkspaceTreeError> {
        if WorkspaceIdentity::new(&worktree_path) != self.workspace_identity {
            return Err(WorkspaceTreeError::IdentityMismatch);
        }
        if self.workflow_node(&execution_id).is_some() {
            return Ok(());
        }
        let sibling_order = self.next_sibling_order(None);
        let status = WorkspaceNodeStatus::Running;
        let completion_signals = NodeCompletionSignalState::default();

        self.nodes.push(WorkspaceTreeNode {
            worktree: None,
            id: execution_id.clone(),
            parent_id: None,
            sibling_order,
            kind: WorkspaceNodeKind::Workflow,
            title: non_empty_or(workflow_name, DEFAULT_WORKFLOW_TITLE),
            status,
            status_classification: WorkspaceNodeStatusClassification::Idle,
            delegate_waits_for_child: false,
            background_failure: false,
            activity: None,
            error_reason: None,
            updated_at_bits: timestamp.to_bits(),
            execution_id: Some(execution_id.clone()),
            node_execution_id: None,
            node_name: None,
            attempt: None,
            execution_parent: None,
            retry_predecessor_id: None,
            past_attempt_ids: Vec::new(),
            is_retry_history: false,
            completion_signals,
            has_artifact: false,
            session_id: None,
            model: None,
            provider: None,
            can_rename: false,
            can_approve: false,
            can_retry: false,
            can_resume_session: false,
            process_presence: Default::default(),
            can_abort: true,
            can_archive: true,
            display_command: None,
            shell: None,
            cwd: None,
            command_result: None,
            dynamic_fanout: false,
        });
        // These compact rule records are aggregate state, not visible Nodes.
        // They preserve dynamic/static fanout identity across later commits
        // without retaining the full Workflow definition.
        for name in dynamic_fanout_names {
            let status = WorkspaceNodeStatus::Waiting;
            let completion_signals = NodeCompletionSignalState::default();

            self.nodes.push(WorkspaceTreeNode {
                worktree: None,
                id: dynamic_fanout_sentinel_id(&execution_id, &name),
                parent_id: Some(execution_id.clone()),
                sibling_order: INTERNAL_SIBLING_ORDER,
                kind: WorkspaceNodeKind::Fanout,
                title: name,
                status,
                status_classification: WorkspaceNodeStatusClassification::Idle,
                delegate_waits_for_child: false,
                background_failure: false,
                activity: None,
                error_reason: None,
                updated_at_bits: timestamp.to_bits(),
                execution_id: Some(execution_id.clone()),
                node_execution_id: None,
                node_name: None,
                attempt: None,
                execution_parent: None,
                retry_predecessor_id: None,
                past_attempt_ids: Vec::new(),
                is_retry_history: false,
                completion_signals,
                has_artifact: false,
                session_id: None,
                model: None,
                provider: None,
                can_rename: false,
                can_approve: false,
                can_retry: false,
                can_resume_session: false,
                process_presence: Default::default(),
                can_abort: false,
                can_archive: false,
                display_command: None,
                shell: None,
                cwd: None,
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
        parent: Option<ExecutionParentRef>,
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

        let (parent_id, semantic_key) = if let Some(parent) = &parent {
            // 実行木の親（合成子インスタンス）の tree node へぶら下げる。
            let parent_node = self
                .nodes
                .iter()
                .find(|node| {
                    node.execution_id.as_deref() == Some(execution_id.as_str())
                        && node.node_execution_id.as_deref() == Some(parent.parent_id.as_str())
                })
                .ok_or_else(|| WorkspaceTreeError::MissingParent(parent.parent_id.clone()))?;
            let parent_tree_id = parent_node.id.clone();
            let parent_dynamic = parent_node.dynamic_fanout;
            let prior = self
                .nodes
                .iter()
                .filter(|node| {
                    node.parent_id.as_deref() == Some(parent_tree_id.as_str())
                        && node.node_name.as_deref() == Some(node_name.as_str())
                })
                .count();
            let semantic_key = node_semantic_key(
                &execution_id,
                &node_name,
                kind,
                Some(parent),
                prior,
                parent_dynamic,
            );
            (parent_tree_id, semantic_key)
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
            let semantic_key =
                node_semantic_key(&execution_id, &node_name, kind, None, occurrence, false);
            (workflow_id, semantic_key)
        };
        let dynamic_fanout = kind == NodeKindName::Fanout
            && self
                .nodes
                .iter()
                .any(|node| node.id == dynamic_fanout_sentinel_id(&execution_id, &node_name));
        let sibling_order = self.next_sibling_order(Some(&parent_id));
        let status = WorkspaceNodeStatus::Running;
        let completion_signals = NodeCompletionSignalState::default();

        let workspace_kind = WorkspaceNodeKind::from(kind);
        let activity = (kind == NodeKindName::Session).then(AgentSessionActivity::default);
        let mut node = WorkspaceTreeNode {
            worktree: None,
            id: String::new(),
            parent_id: Some(parent_id),
            sibling_order,
            kind: workspace_kind,
            title: node_name.clone(),
            status,
            status_classification: WorkspaceNodeStatusClassification::Idle,
            delegate_waits_for_child: false,
            background_failure: false,
            activity,
            error_reason: None,
            updated_at_bits: timestamp.to_bits(),
            execution_id: Some(execution_id),
            node_execution_id: Some(node_execution_id),
            node_name: Some(node_name),
            attempt: Some(attempt),
            execution_parent: parent,
            retry_predecessor_id: None,
            past_attempt_ids: Vec::new(),
            is_retry_history: false,
            completion_signals,
            has_artifact: false,
            session_id: None,
            model: None,
            provider: None,
            can_rename: false,
            can_approve: false,
            can_retry: false,
            can_resume_session: false,
            process_presence: Default::default(),
            can_abort: false,
            can_archive: false,
            display_command: None,
            shell: None,
            cwd: None,
            command_result: None,
            dynamic_fanout,
        };
        node.id = node_identity(
            node.execution_id
                .as_deref()
                .expect("execution id was assigned"),
            kind,
            &semantic_key,
            node.is_standalone_session_root(),
        )?;
        self.nodes.push(node);
        Ok(())
    }

    pub(super) fn recompute_branch_summaries(&mut self) {
        let non_rename_public_root_ids = WorkspacePublicRoot::all(&self.nodes)
            .into_iter()
            .filter(|root| !root.node().is_standalone_session_root())
            .map(|root| root.node().id.clone())
            .collect::<HashSet<_>>();
        for node in &mut self.nodes {
            node.can_rename = super::value_objects::session_can_rename(
                node.kind,
                node.session_id.is_some(),
                non_rename_public_root_ids.contains(&node.id),
                node.is_standalone_session_root(),
            );
        }
        let mut by_parent = BTreeMap::<String, Vec<usize>>::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.is_internal_rule_record() && !node.is_retry_history {
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
                    WorkspaceNodeKind::Fanout
                        | WorkspaceNodeKind::Workflow
                        | WorkspaceNodeKind::Sequence
                ) && !node.is_internal_rule_record()
            })
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        branch_ids.reverse();
        for branch_id in branch_ids {
            let Some(children) = by_parent.get(&branch_id) else {
                continue;
            };
            let child_updated = children
                .iter()
                .map(|index| self.nodes[*index].updated_at_bits)
                .max_by(|left, right| f64::from_bits(*left).total_cmp(&f64::from_bits(*right)));
            if let Some(branch) = self.nodes.iter_mut().find(|node| node.id == branch_id) {
                if let Some(updated) = child_updated {
                    branch.updated_at_bits = max_f64_bits(branch.updated_at_bits, updated);
                }
            }
        }
        self.recompute_status_classifications();
    }

    pub(super) fn recompute_status_classifications(&mut self) {
        self.aggregate_status_classifications();
    }

    fn aggregate_status_classifications(&mut self) {
        aggregate_status_classifications(&mut self.nodes);
    }

    fn recompute_retry_histories(&mut self) {
        for node in &mut self.nodes {
            node.past_attempt_ids.clear();
            node.is_retry_history = false;
        }
        let chains = crate::domain::workflow::services::history::retry_history_chains(
            self.nodes.iter().filter_map(|node| {
                Some((
                    node.node_execution_id.as_deref()?,
                    node.retry_predecessor_id.as_deref(),
                    matches!(
                        node.status,
                        WorkspaceNodeStatus::Completed | WorkspaceNodeStatus::Aborted
                    ),
                ))
            }),
        );
        for (head, chain) in chains {
            let past_attempt_ids: Vec<_> = chain
                .iter()
                .filter_map(|id| {
                    self.nodes
                        .iter()
                        .find(|node| node.node_execution_id.as_ref() == Some(id))
                        .map(|node| node.id.clone())
                })
                .collect();
            for node in &mut self.nodes {
                if node.node_execution_id.as_ref() == Some(&head) {
                    node.past_attempt_ids = past_attempt_ids.clone();
                }
                if node
                    .node_execution_id
                    .as_ref()
                    .is_some_and(|id| chain.contains(id))
                {
                    node.is_retry_history = true;
                }
            }
        }
    }

    pub(super) fn recompute_root_order(&mut self) {
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
        workflows.sort_by(by_title);
        for (order, (index, _, _)) in workflows.into_iter().enumerate() {
            self.nodes[index].sibling_order = order as u64;
        }
    }
}

pub struct WorkspaceTreeProjector;

impl WorkspaceTreeProjector {
    pub fn project(
        tree: &mut WorkspaceTree,
        facts: impl IntoIterator<Item = WorkspaceStructureFact>,
    ) -> Result<bool, WorkspaceTreeError> {
        let mut applied_fact = false;
        for fact in facts {
            applied_fact = true;
            match fact {
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id,
                    workflow_name,
                    worktree_path,
                    dynamic_fanout_names,
                    timestamp,
                } => tree.apply_workflow_started(
                    execution_id,
                    workflow_name,
                    worktree_path,
                    dynamic_fanout_names,
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
                        workflow.can_abort = status.can_abort();
                        workflow.can_archive = true;
                    }
                }
                WorkspaceStructureFact::NodeStarted {
                    execution_id,
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    parent,
                    timestamp,
                } => tree.apply_node_started(
                    execution_id,
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    parent,
                    timestamp,
                )?,
                WorkspaceStructureFact::NodeRetryLinked {
                    execution_id,
                    node_execution_id,
                    predecessor_node_execution_id,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.retry_predecessor_id = Some(predecessor_node_execution_id);
                }
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
                WorkspaceStructureFact::NodeActivityProjected {
                    execution_id,
                    node_execution_id,
                    activity,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    if node.kind != WorkspaceNodeKind::WorkflowSession {
                        return Err(WorkspaceTreeError::InvalidNode(node.id.clone()));
                    }
                    node.activity = Some(activity);
                }
                WorkspaceStructureFact::NodeSessionDisplayNameProjected {
                    execution_id,
                    node_execution_id,
                    manual_name,
                    provider_session_title,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    if node.kind != WorkspaceNodeKind::WorkflowSession {
                        return Err(WorkspaceTreeError::InvalidNode(node.id.clone()));
                    }
                    node.title = super::value_objects::session_node_title(
                        manual_name,
                        provider_session_title,
                        node.node_name
                            .clone()
                            .ok_or_else(|| WorkspaceTreeError::InvalidNode(node.id.clone()))?,
                        node.is_standalone_session_root(),
                    );
                }
                WorkspaceStructureFact::NodeCommandPrepared {
                    execution_id,
                    node_execution_id,
                    display_command,
                    shell,
                    cwd,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.display_command = Some(display_command);
                    node.shell = shell;
                    node.cwd = cwd;
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
                WorkspaceStructureFact::NodeArtifactProduced {
                    execution_id,
                    node_execution_id,
                    command_result_candidate,
                    timestamp,
                } => {
                    let node = tree
                        .execution_node_mut(&execution_id, &node_execution_id)
                        .ok_or_else(|| {
                            WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                        })?;
                    node.has_artifact = true;
                    if node.kind == WorkspaceNodeKind::WorkflowCommand {
                        node.command_result = command_result_candidate;
                    }
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
                    if failure_kind == NodeExecutionFailureKind::UserAbort {
                        update_node_state(
                            tree,
                            &execution_id,
                            &node_execution_id,
                            WorkspaceNodeStatus::Aborted,
                            Some("Workflow node aborted".into()),
                            timestamp,
                        )?;
                    } else {
                        let node = tree
                            .execution_node_mut(&execution_id, &node_execution_id)
                            .ok_or_else(|| {
                                WorkspaceTreeError::MissingNodeExecution(node_execution_id.clone())
                            })?;
                        node.updated_at_bits =
                            max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                    }
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
                    node.can_approve = node.status.can_approve();
                    node.can_retry = false;
                    node.updated_at_bits = max_f64_bits(node.updated_at_bits, timestamp.to_bits());
                }
            }
        }
        tree.recompute_root_order();
        tree.recompute_retry_histories();
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
    node.can_approve = status.can_approve();
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
        WorkspaceNodeKind::Workflow => {
            node.parent_id.is_none()
                && has_execution
                && node.node_execution_id.is_none()
                && node.node_name.is_none()
                && node.attempt.is_none()
                && node.session_id.is_none()
                && node.display_command.is_none()
                && node.command_result.is_none()
                && node.activity.is_none()
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
                && node.display_command.is_none()
                && node.command_result.is_none()
                && node.activity.is_none()
        }
        WorkspaceNodeKind::Sequence => {
            node.parent_id.is_some()
                && has_execution
                && has_node_execution
                && has_node_name
                && node.attempt.is_some()
                && !node.dynamic_fanout
                && node.session_id.is_none()
                && node.display_command.is_none()
                && node.command_result.is_none()
                && node.activity.is_none()
        }
        WorkspaceNodeKind::WorkflowSession => {
            node.parent_id.is_some()
                && has_execution
                && has_node_execution
                && has_node_name
                && node.attempt.is_some()
                && node.display_command.is_none()
                && node.command_result.is_none()
                && node.activity.is_some()
        }
        WorkspaceNodeKind::WorkflowCommand => {
            node.parent_id.is_some()
                && has_execution
                && has_node_execution
                && has_node_name
                && node.attempt.is_some()
                && node.session_id.is_none()
                && node.activity.is_none()
        }
    }
}

pub(super) fn workflow_status(status: ExecutionStatus) -> WorkspaceNodeStatus {
    match status {
        ExecutionStatus::Running => WorkspaceNodeStatus::Running,
        ExecutionStatus::Completed => WorkspaceNodeStatus::Completed,
        ExecutionStatus::Aborted => WorkspaceNodeStatus::Aborted,
    }
}

pub(crate) fn observe_background_failures<'a>(
    nodes: &mut [WorkspaceTreeNode],
    observations: impl IntoIterator<Item = (&'a str, &'a str)>,
) {
    for (target, message) in observations {
        for node in nodes.iter_mut() {
            if super::value_objects::observes_node(
                &node.id,
                node.execution_id.as_deref(),
                node.node_execution_id.as_deref(),
                target,
            ) {
                node.observe_background_failure(message);
            }
        }
    }
    aggregate_status_classifications(nodes);
}

fn aggregate_status_classifications(nodes: &mut [WorkspaceTreeNode]) {
    let inputs = nodes
        .iter()
        .map(|node| {
            let mut input = node.classification_input();
            if let Some(parent) = nodes.iter().find(|parent| {
                Some(&parent.id) == node.parent_id.as_ref()
                    && parent.kind == WorkspaceNodeKind::WorkflowSession
            }) {
                if let Some(parent_id) = parent.node_execution_id.as_deref() {
                    let continuation = crate::domain::workflow::entities::workflow_execution::delegate::delegate_continuation(
                        parent_id,
                        nodes.iter().filter_map(|node| {
                            Some((node.node_execution_id.as_deref()?, node.retry_predecessor_id.as_deref()))
                        }),
                    );
                    input.parent_id = nodes.iter()
                        .find(|node| node.node_execution_id.as_deref() == Some(continuation))
                        .map(|node| node.id.clone());
                }
            }
            input
        })
        .collect::<Vec<_>>();
    let classifications = super::WorkspaceNodeClassification::aggregate_all(&inputs);
    for node in nodes {
        node.status_classification = classifications[&node.id];
    }
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

fn sequence_child_occurrence_key(
    parent_node_execution_id: &str,
    node_name: &str,
    occurrence: usize,
) -> String {
    format!("scope:{parent_node_execution_id}:node:{node_name}:occurrence:{occurrence}")
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

fn sequence_branch_occurrence_key(
    execution_id: &str,
    sequence_name: &str,
    occurrence: usize,
) -> String {
    if occurrence == 0 {
        format!("workflow\0{execution_id}\0sequence\0{sequence_name}")
    } else {
        format!("workflow\0{execution_id}\0sequence\0{sequence_name}\0occurrence\0{occurrence}")
    }
}

fn fanout_child_occurrence_key(
    parent_node_execution_id: &str,
    item_index: Option<usize>,
    child_index: usize,
    child_name: &str,
    occurrence: usize,
) -> String {
    format!(
        "fanout-child\0{parent_node_execution_id}\0{item_index:?}\0{child_index}\0{child_name}\0{occurrence}"
    )
}

fn fanout_dynamic_child_occurrence_key(
    parent_node_execution_id: &str,
    child_index: usize,
    child_name: &str,
    occurrence: usize,
) -> String {
    fanout_child_occurrence_key(
        parent_node_execution_id,
        None,
        child_index,
        child_name,
        occurrence,
    )
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

fn opaque_standalone_session_node_id(execution_id: &str, semantic_key: &str) -> String {
    opaque_id("node-s", &format!("{execution_id}\0{semantic_key}"))
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
#[path = "mod_test.rs"]
mod mod_tests;

fn node_identity(
    execution_id: &str,
    kind: NodeKindName,
    semantic_key: &str,
    standalone: bool,
) -> Result<String, WorkspaceTreeError> {
    match kind {
        NodeKindName::Fanout | NodeKindName::Sequence => Ok(opaque_branch_id(semantic_key)),
        NodeKindName::Session | NodeKindName::Command => {
            if standalone {
                Ok(opaque_standalone_session_node_id(
                    execution_id,
                    semantic_key,
                ))
            } else {
                opaque_workflow_node_id(execution_id, semantic_key)
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct NodeIdentityIndex {
    pub identities: std::collections::HashMap<String, String>,
    counts: std::collections::HashMap<(Option<String>, String), usize>,
    dynamic: HashSet<String>,
    dynamic_parents: HashSet<String>,
    roots: Vec<(String, String, u32)>,
}

impl NodeIdentityIndex {
    pub fn record(
        &mut self,
        record: &crate::domain::workflow::NodeFactRecord,
    ) -> Result<(), WorkspaceTreeError> {
        use crate::domain::workflow::NodeFact;
        let NodeFact::Started(started) = &record.fact else {
            return Ok(());
        };
        if let Some(definition) = started
            .root
            .as_ref()
            .and_then(|root| root.definition.as_ref())
        {
            self.dynamic = definition.dynamic_fanout_names().into_iter().collect();
        }
        self.record_start_identity(&record.meta, started.parent.as_ref())
    }

    pub(crate) fn record_start_identity(
        &mut self,
        meta: &crate::domain::workflow::NodeFactMeta,
        parent: Option<&ExecutionParentRef>,
    ) -> Result<(), WorkspaceTreeError> {
        if self.identities.contains_key(&meta.node_execution_id) {
            return Ok(());
        }

        let count = self
            .counts
            .entry((
                parent.map(|parent| parent.parent_id.clone()),
                meta.node_name.clone(),
            ))
            .or_default();
        let key = node_semantic_key(
            &meta.tree_id,
            &meta.node_name,
            meta.kind,
            parent,
            *count,
            parent.is_some_and(|parent| self.dynamic_parents.contains(&parent.parent_id)),
        );
        *count += 1;
        if meta.kind == NodeKindName::Fanout && self.dynamic.contains(&meta.node_name) {
            self.dynamic_parents.insert(meta.node_execution_id.clone());
        }
        self.identities.insert(
            meta.node_execution_id.clone(),
            node_identity(
                &meta.tree_id,
                meta.kind,
                &key,
                meta.kind == NodeKindName::Session
                    && meta.node_execution_id == meta.tree_id
                    && parent.is_none(),
            )?,
        );
        if parent.is_none() {
            self.roots.push((
                meta.node_execution_id.clone(),
                meta.node_name.clone(),
                meta.attempt,
            ));
        }
        Ok(())
    }

    pub fn public_root(&self) -> Option<&str> {
        super::services::select_public_root(self.roots.iter().enumerate().map(
            |(order, (id, name, attempt))| {
                (
                    id.as_str(),
                    order as u64,
                    !self
                        .roots
                        .iter()
                        .any(|(_, other, next)| other == name && next > attempt),
                )
            },
        ))
    }
}

pub(crate) fn node_identities(
    records: &[crate::domain::workflow::NodeFactRecord],
) -> Result<std::collections::HashMap<String, String>, WorkspaceTreeError> {
    let mut index = NodeIdentityIndex::default();
    for record in records {
        index.record(record)?;
    }
    Ok(index.identities)
}

fn node_semantic_key(
    execution: &str,
    name: &str,
    kind: NodeKindName,
    parent: Option<&ExecutionParentRef>,
    occurrence: usize,
    dynamic: bool,
) -> String {
    if let Some(parent) = parent {
        match parent.fanout_slot() {
            Some(slot) if dynamic => fanout_dynamic_child_occurrence_key(
                &parent.parent_id,
                slot.child_index,
                name,
                occurrence,
            ),
            Some(slot) => fanout_child_occurrence_key(
                &parent.parent_id,
                slot.item_index,
                slot.child_index,
                name,
                occurrence,
            ),
            None => sequence_child_occurrence_key(&parent.parent_id, name, occurrence),
        }
    } else {
        match kind {
            NodeKindName::Fanout => fanout_branch_occurrence_key(execution, name, occurrence),
            NodeKindName::Sequence => sequence_branch_occurrence_key(execution, name, occurrence),
            _ => workflow_node_occurrence_key(name, occurrence),
        }
    }
}

pub(crate) fn public_root_node(
    records: &[crate::domain::workflow::NodeFactRecord],
) -> Option<&str> {
    use crate::domain::workflow::NodeFact;
    super::services::select_public_root(records.iter().enumerate().filter_map(|(order, record)| {
        let NodeFact::Started(started) = &record.fact else {
            return None;
        };
        Some((
            record.meta.node_execution_id.as_str(),
            order as u64,
            started.parent.is_none()
                && !records.iter().any(|other| {
                    matches!(other.fact, NodeFact::Started(_))
                        && other.meta.node_name == record.meta.node_name
                        && other.meta.parent_id == record.meta.parent_id
                        && other.meta.attempt > record.meta.attempt
                }),
        ))
    }))
}
