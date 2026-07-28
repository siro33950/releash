use crate::domain::workflow::{
    ExecutionStatus, FanoutParentRef, NodeExecutionFailureKind, NodeKindName, WorkflowDefinition,
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
pub enum WorkspaceSessionListKind {
    Active,
    Closed,
    Archived,
}

impl WorkspaceSessionListKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeKind {
    Session,
    Workflow,
    Fanout,
    WorkflowSession,
    WorkflowCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeStatus {
    Running,
    Failed,
    Error,
    Waiting,
    Interrupted,
    Aborted,
    Completed,
}

impl WorkspaceNodeStatus {
    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Error => "error",
            Self::Waiting => "waiting",
            Self::Interrupted => "interrupted",
            Self::Aborted => "aborted",
            Self::Completed => "completed",
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
    pub id: String,
    pub parent_id: Option<String>,
    pub sibling_order: u64,
    pub kind: WorkspaceNodeKind,
    pub title: String,
    pub status: WorkspaceNodeStatus,
    pub error_reason: Option<String>,
    pub updated_at_bits: u64,
    pub execution_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub node_name: Option<String>,
    pub attempt: Option<u32>,
    pub session_id: Option<String>,
    pub can_approve: bool,
    pub can_close: bool,
    pub can_stop: bool,
    pub can_resume: bool,
    /// Recovery fence owned by this node's source identity. For Workflow
    /// roots this is the execution owner; for bound Workflow Session leaves
    /// it is the Session owner. The public resume reason is derived from
    /// these owner-local values after every projection.
    pub recovery_owner_reason: Option<String>,
    pub resume_unavailable_reason: Option<String>,
    pub can_abort: bool,
    pub can_archive: bool,
    pub display_command: Option<String>,
    pub command_result: Option<WorkspaceCommandResult>,
    /// Aggregate-only rule used for established fanout occurrence IDs.
    pub dynamic_fanout: bool,
}

impl WorkspaceTreeNode {
    pub fn updated_at(&self) -> f64 {
        f64::from_bits(self.updated_at_bits)
    }

    pub fn is_leaf(&self) -> bool {
        matches!(
            self.kind,
            WorkspaceNodeKind::Session
                | WorkspaceNodeKind::WorkflowSession
                | WorkspaceNodeKind::WorkflowCommand
        )
    }

    pub fn is_internal_rule_record(&self) -> bool {
        self.kind == WorkspaceNodeKind::Fanout
            && self.sibling_order == INTERNAL_SIBLING_ORDER
            && self.node_execution_id.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSessionFact {
    pub id: String,
    pub worktree_path: String,
    pub state: WorkspaceSessionState,
    pub error_reason: Option<String>,
    pub updated_at_bits: u64,
    pub title: Option<String>,
    pub first_message: String,
    pub workflow_node_session: bool,
    pub workflow_execution_id: Option<String>,
    pub workflow_node_execution_id: Option<String>,
    pub unresolved_recovery_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceStructureFact {
    SessionProjected(WorkspaceSessionFact),
    SessionRemoved {
        session_id: String,
    },
    WorkflowStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        definition: WorkflowDefinition,
        timestamp: f64,
    },
    WorkflowSummaryProjected {
        execution_id: String,
        workflow_name: String,
        status: ExecutionStatus,
        updated_at: f64,
    },
    WorkflowRemoved {
        execution_id: String,
    },
    RecoveryFenceProjected {
        owner: String,
        reason: Option<String>,
    },
    NodeStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        timestamp: f64,
    },
    NodeAgentBound {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    NodeCommandPrepared {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        timestamp: f64,
    },
    NodeCommandResult {
        execution_id: String,
        node_execution_id: String,
        result: Option<WorkspaceCommandResult>,
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
    NodeApprovalResolved {
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
