mod node_timestamp;
pub(crate) use node_timestamp::NodeTimestampObservation;

use crate::domain::workflow::{
    AgentSessionActivity, ExecutionParentRef, ExecutionStatus, NodeCompletionSignalState,
    NodeExecutionFailureKind, NodeKindName, NodeProcessPresence,
};

pub(super) const INTERNAL_SIBLING_ORDER: u64 = i64::MAX as u64;

/// Canonical identity used to partition a Workspace query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceIdentity(String);

impl WorkspaceIdentity {
    pub fn new(value: impl AsRef<str>) -> Self {
        Self(crate::domain::repository::normalize_repo_path(
            value.as_ref(),
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeKind {
    Workflow,
    Fanout,
    /// 部品 sequence の実行インスタンス（実行木の branch）。
    Sequence,
    WorkflowSession,
    WorkflowCommand,
}

impl From<NodeKindName> for WorkspaceNodeKind {
    fn from(kind: NodeKindName) -> Self {
        match kind {
            NodeKindName::Session => Self::WorkflowSession,
            NodeKindName::Command => Self::WorkflowCommand,
            NodeKindName::Fanout => Self::Fanout,
            NodeKindName::Sequence => Self::Sequence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeStatus {
    Running,
    Waiting,
    Aborted,
    Completed,
}

impl From<crate::domain::workflow::NodeExecutionStatus> for WorkspaceNodeStatus {
    fn from(status: crate::domain::workflow::NodeExecutionStatus) -> Self {
        use crate::domain::workflow::NodeExecutionStatus as S;
        match status {
            S::Running => Self::Running,
            S::WaitingApproval => Self::Waiting,
            S::Succeeded => Self::Completed,
            S::Aborted => Self::Aborted,
        }
    }
}

impl WorkspaceNodeStatus {
    pub fn can_approve(self) -> bool {
        self == Self::Waiting
    }

    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Aborted => "aborted",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeStatusClassification {
    Active,
    Attention,
    Idle,
}

impl WorkspaceNodeStatusClassification {
    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Attention => "attention",
            Self::Idle => "idle",
        }
    }

    pub(super) fn most_severe(self, other: Self) -> Self {
        if self.severity() >= other.severity() {
            self
        } else {
            other
        }
    }

    fn severity(self) -> u8 {
        match self {
            Self::Attention => 3,
            Self::Active => 2,
            Self::Idle => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCommandResult {
    pub exit_code: i64,
    pub duration: u64,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTreeNode {
    pub worktree: Option<crate::domain::workflow::IsolatedWorktree>,
    pub id: String,
    pub parent_id: Option<String>,
    pub sibling_order: u64,
    pub kind: WorkspaceNodeKind,
    pub title: String,
    pub status: WorkspaceNodeStatus,
    pub process_presence: NodeProcessPresence,
    pub status_classification: WorkspaceNodeStatusClassification,
    pub delegate_waits_for_child: bool,
    pub background_failure: bool,
    pub activity: Option<AgentSessionActivity>,
    pub error_reason: Option<String>,
    pub updated_at_bits: u64,
    pub execution_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub node_name: Option<String>,
    pub attempt: Option<u32>,
    pub execution_parent: Option<ExecutionParentRef>,
    pub retry_predecessor_id: Option<String>,
    pub past_attempt_ids: Vec<String>,
    pub is_retry_history: bool,
    pub completion_signals: NodeCompletionSignalState,
    pub has_artifact: bool,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<crate::domain::provider_lifecycle::ProviderKind>,
    pub can_rename: bool,
    pub can_approve: bool,
    pub can_retry: bool,
    pub can_resume_session: bool,
    pub can_abort: bool,
    pub can_archive: bool,
    pub display_command: Option<String>,
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub command_result: Option<WorkspaceCommandResult>,
    /// Aggregate-only rule used for established fanout occurrence IDs.
    pub dynamic_fanout: bool,
}

pub(crate) fn node_failure_reason(
    reason: Option<&str>,
    command_result: Option<&WorkspaceCommandResult>,
) -> Option<String> {
    reason.map(str::to_string).or_else(|| {
        command_result
            .filter(|result| result.exit_code != 0)
            .map(|result| format!("Command exited with code {}", result.exit_code))
    })
}

pub(crate) fn aborted_node_reason(
    status: crate::domain::workflow::NodeExecutionStatus,
) -> Option<&'static str> {
    (status == crate::domain::workflow::NodeExecutionStatus::Aborted)
        .then_some("Workflow node aborted")
}

impl WorkspaceTreeNode {
    pub fn failure_reason(&self) -> Option<String> {
        node_failure_reason(self.error_reason.as_deref(), self.command_result.as_ref())
    }

    pub fn updated_at(&self) -> f64 {
        f64::from_bits(self.updated_at_bits)
    }

    pub fn is_leaf(&self) -> bool {
        matches!(
            self.kind,
            WorkspaceNodeKind::WorkflowSession | WorkspaceNodeKind::WorkflowCommand
        )
    }

    pub fn is_standalone_session_root(&self) -> bool {
        self.kind == WorkspaceNodeKind::WorkflowSession
            && self.node_execution_id.is_some()
            && self.node_execution_id == self.execution_id
            && self.parent_id == self.execution_id
    }

    pub fn is_internal_rule_record(&self) -> bool {
        self.kind == WorkspaceNodeKind::Fanout
            && self.sibling_order == INTERNAL_SIBLING_ORDER
            && self.node_execution_id.is_none()
    }

    pub(super) fn classify_status(
        &self,
        children: impl IntoIterator<Item = WorkspaceNodeStatusClassification>,
    ) -> WorkspaceNodeStatusClassification {
        self.classification_input().classify(children)
    }

    pub(crate) fn classification_input(&self) -> WorkspaceNodeClassification {
        WorkspaceNodeClassification {
            id: self.id.clone(),
            parent_id: self.parent_id.clone(),
            execution_id: self.execution_id.clone(),
            node_execution_id: self.node_execution_id.clone(),
            kind: self.kind,
            status: self.status,
            process_presence: self.process_presence,
            delegate_waits_for_child: self.delegate_waits_for_child,
            background_failure: self.background_failure,
            activity: self.activity,
            has_session: self.session_id.is_some(),
            completion_signals: self.completion_signals,
            hidden: self.is_internal_rule_record() || self.is_retry_history,
        }
    }
}

/// Workspace に属する 1 本の実行木の状態。木の節ではなく、実行そのものの事実を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceExecution {
    pub execution_id: String,
    pub launched_as: crate::domain::workflow::ExecutionTreeLaunch,
    pub worktree_path: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub(crate) updated_at: f64,
    pub archive: Option<crate::domain::workflow::ExecutionTreeArchiveRecord>,
    /// Session として起動した実行木の session。
    pub session: Option<crate::domain::agent_session::aggregates::AgentSession>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceStructureFact {
    WorkflowStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        dynamic_fanout_names: std::collections::BTreeSet<String>,
        timestamp: f64,
    },
    WorkflowSummaryProjected {
        execution_id: String,
        workflow_name: String,
        status: ExecutionStatus,
        updated_at: f64,
    },
    NodeStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        parent: Option<ExecutionParentRef>,
        timestamp: f64,
    },
    NodeRetryLinked {
        execution_id: String,
        node_execution_id: String,
        predecessor_node_execution_id: String,
    },
    NodeAgentBound {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    NodeActivityProjected {
        execution_id: String,
        node_execution_id: String,
        activity: AgentSessionActivity,
    },
    NodeSessionDisplayNameProjected {
        execution_id: String,
        node_execution_id: String,
        manual_name: Option<String>,
        provider_session_title: Option<String>,
    },
    NodeCommandPrepared {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        shell: Option<String>,
        cwd: Option<String>,
        timestamp: f64,
    },
    NodeArtifactProduced {
        execution_id: String,
        node_execution_id: String,
        command_result_candidate: Option<WorkspaceCommandResult>,
        timestamp: f64,
    },
    NodeCompleted {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeFailed {
        execution_id: String,
        node_execution_id: String,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
        timestamp: f64,
    },
    NodeApprovalRequested {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTreeError {
    IdentityMismatch,
    DuplicateNode(String),
    DuplicateSession(String),
    DuplicateNodeExecution(String),
    DuplicateSiblingOrder(String),
    MissingParent(String),
    MissingWorkflow(String),
    MissingNodeExecution(String),
    InvalidParent(String),
    InvalidNode(String),
    ParentCycle(String),
}

impl std::fmt::Display for WorkspaceTreeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdentityMismatch => formatter.write_str("workspace identity mismatch"),
            Self::DuplicateNode(id) => write!(formatter, "duplicate Workspace node: {id}"),
            Self::DuplicateSession(id) => {
                write!(formatter, "duplicate Workspace Session binding: {id}")
            }
            Self::DuplicateNodeExecution(id) => {
                write!(formatter, "duplicate Workspace node execution: {id}")
            }
            Self::DuplicateSiblingOrder(id) => {
                write!(formatter, "duplicate Workspace sibling order: {id}")
            }
            Self::MissingParent(id) => write!(formatter, "Workspace parent is missing: {id}"),
            Self::MissingWorkflow(id) => write!(formatter, "Workspace Workflow is missing: {id}"),
            Self::MissingNodeExecution(id) => {
                write!(formatter, "Workspace node execution is missing: {id}")
            }
            Self::InvalidParent(id) => write!(formatter, "invalid Workspace parent: {id}"),
            Self::InvalidNode(id) => write!(formatter, "invalid Workspace node: {id}"),
            Self::ParentCycle(id) => write!(formatter, "Workspace parent cycle: {id}"),
        }
    }
}

impl std::error::Error for WorkspaceTreeError {}

impl WorkspaceTreeNode {
    pub(super) fn observe_background_failure(&mut self, message: &str) {
        self.error_reason = Some(message.into());
        self.background_failure = true;
        self.status_classification = self.classify_status([]);
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
pub(crate) mod mod_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeClassification {
    pub id: String,
    pub parent_id: Option<String>,
    pub execution_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub kind: WorkspaceNodeKind,
    pub status: WorkspaceNodeStatus,
    pub process_presence: NodeProcessPresence,
    pub delegate_waits_for_child: bool,
    pub background_failure: bool,
    pub activity: Option<AgentSessionActivity>,
    pub has_session: bool,
    pub completion_signals: NodeCompletionSignalState,
    pub hidden: bool,
}

impl WorkspaceNodeClassification {
    pub(crate) fn classify(
        &self,
        children: impl IntoIterator<Item = WorkspaceNodeStatusClassification>,
    ) -> WorkspaceNodeStatusClassification {
        use WorkspaceNodeStatusClassification as C;

        let session = if self.kind == WorkspaceNodeKind::WorkflowSession {
            Some(if !self.has_session {
                C::Active
            } else if self.process_presence == NodeProcessPresence::ConfirmedAbsent {
                C::Idle
            } else {
                match self.activity.unwrap_or_default() {
                    AgentSessionActivity::Working => C::Active,
                    AgentSessionActivity::AwaitingAnswer => C::Attention,
                    AgentSessionActivity::AwaitingInstruction => C::Idle,
                }
            })
        } else {
            None
        };
        let node = if self.background_failure || self.status == WorkspaceNodeStatus::Waiting {
            Some(C::Attention)
        } else if matches!(
            self.status,
            WorkspaceNodeStatus::Completed | WorkspaceNodeStatus::Aborted
        ) {
            Some(C::Idle)
        } else if self.delegate_waits_for_child {
            None
        } else if self.kind == WorkspaceNodeKind::WorkflowCommand
            && self.process_presence == NodeProcessPresence::ConfirmedAbsent
        {
            Some(C::Attention)
        } else if self.kind == WorkspaceNodeKind::WorkflowSession && self.has_session {
            match (
                self.process_presence,
                self.activity,
                self.completion_signals,
            ) {
                (NodeProcessPresence::ConfirmedAbsent, _, _) => Some(C::Attention),
                (
                    _,
                    Some(AgentSessionActivity::AwaitingInstruction),
                    NodeCompletionSignalState::Pending | NodeCompletionSignalState::StopReceived,
                ) => Some(C::Attention),
                _ => Some(C::Active),
            }
        } else {
            Some(C::Active)
        };
        node.into_iter()
            .chain(session)
            .chain(children)
            .fold(C::Idle, C::most_severe)
    }
    pub(crate) fn aggregate_all(
        nodes: &[Self],
    ) -> std::collections::HashMap<String, WorkspaceNodeStatusClassification> {
        let mut by_parent = std::collections::BTreeMap::<&str, Vec<usize>>::new();
        for (index, node) in nodes.iter().enumerate().filter(|(_, node)| !node.hidden) {
            if let Some(parent) = node.parent_id.as_deref() {
                by_parent.entry(parent).or_default().push(index);
            }
        }
        fn visit(
            index: usize,
            nodes: &[WorkspaceNodeClassification],
            by_parent: &std::collections::BTreeMap<&str, Vec<usize>>,
            values: &mut [Option<WorkspaceNodeStatusClassification>],
        ) -> WorkspaceNodeStatusClassification {
            if let Some(value) = values[index] {
                return value;
            }
            let node = &nodes[index];
            values[index] = Some(node.classify([]));
            let children = by_parent
                .get(node.id.as_str())
                .into_iter()
                .flatten()
                .map(|&child| visit(child, nodes, by_parent, values))
                .collect::<Vec<_>>();
            let value = node.classify(children);
            values[index] = Some(value);
            value
        }
        let mut values = vec![None; nodes.len()];
        (0..nodes.len())
            .map(|index| {
                (
                    nodes[index].id.clone(),
                    visit(index, nodes, &by_parent, &mut values),
                )
            })
            .collect()
    }
}

pub(crate) fn session_node_title(
    manual: Option<String>,
    provider: Option<String>,
    node_name: String,
    standalone: bool,
) -> String {
    manual
        .or_else(|| standalone.then_some(provider).flatten())
        .unwrap_or(node_name)
}

pub(crate) fn session_can_rename(
    kind: WorkspaceNodeKind,
    has_session: bool,
    public_root: bool,
    standalone: bool,
) -> bool {
    kind == WorkspaceNodeKind::WorkflowSession && has_session && (!public_root || standalone)
}

pub(super) fn observes_node(
    id: &str,
    execution: Option<&str>,
    node_execution: Option<&str>,
    target: &str,
) -> bool {
    id == target || execution == Some(target) || node_execution == Some(target)
}
