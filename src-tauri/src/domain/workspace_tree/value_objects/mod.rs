use crate::domain::workflow::{
    AgentSessionActivity, ExecutionParentRef, ExecutionStatus, NodeCompletionSignalState,
    NodeExecutionFailureKind, NodeKindName, WorkflowDefinition,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceNodeStatus {
    Running,
    Paused,
    Failed,
    Waiting,
    Aborted,
    Completed,
}

impl WorkspaceNodeStatus {
    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Failed => "failed",
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
    Failure,
    Idle,
}

impl WorkspaceNodeStatusClassification {
    pub fn as_public_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Attention => "attention",
            Self::Failure => "failure",
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
            Self::Failure => 3,
            Self::Attention => 2,
            Self::Active => 1,
            Self::Idle => 0,
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
    pub status_classification: WorkspaceNodeStatusClassification,
    pub activity: Option<AgentSessionActivity>,
    pub error_reason: Option<String>,
    pub updated_at_bits: u64,
    pub execution_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub node_name: Option<String>,
    pub attempt: Option<u32>,
    pub retry_predecessor_id: Option<String>,
    pub past_attempt_ids: Vec<String>,
    pub is_retry_history: bool,
    pub completion_signals: NodeCompletionSignalState,
    pub has_artifact: bool,
    pub session_id: Option<String>,
    pub can_approve: bool,
    pub can_retry: bool,
    pub can_close: bool,
    pub can_stop: bool,
    pub can_resume: bool,
    /// Runtime aggregate が resume を受理する leaf。公開 capability は workflow root の
    /// `can_resume` だけであり、この値は root 集約の入力にだけ使う。
    pub(crate) resume_eligible: bool,
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
            WorkspaceNodeKind::WorkflowSession | WorkspaceNodeKind::WorkflowCommand
        )
    }

    pub fn is_internal_rule_record(&self) -> bool {
        self.kind == WorkspaceNodeKind::Fanout
            && self.sibling_order == INTERNAL_SIBLING_ORDER
            && self.node_execution_id.is_none()
    }

    pub(super) fn classify_own_status(
        kind: WorkspaceNodeKind,
        status: WorkspaceNodeStatus,
        activity: Option<AgentSessionActivity>,
        recovery_fenced: bool,
    ) -> WorkspaceNodeStatusClassification {
        if recovery_fenced || status == WorkspaceNodeStatus::Failed {
            WorkspaceNodeStatusClassification::Failure
        } else if matches!(
            status,
            WorkspaceNodeStatus::Completed
                | WorkspaceNodeStatus::Aborted
                | WorkspaceNodeStatus::Paused
        ) {
            WorkspaceNodeStatusClassification::Idle
        } else if kind == WorkspaceNodeKind::WorkflowSession {
            match activity.unwrap_or_default() {
                AgentSessionActivity::Working => WorkspaceNodeStatusClassification::Active,
                AgentSessionActivity::AwaitingAnswer
                | AgentSessionActivity::AwaitingInstruction => {
                    WorkspaceNodeStatusClassification::Attention
                }
            }
        } else if status == WorkspaceNodeStatus::Waiting {
            WorkspaceNodeStatusClassification::Attention
        } else {
            WorkspaceNodeStatusClassification::Active
        }
    }

    pub(super) fn own_status_classification(&self) -> WorkspaceNodeStatusClassification {
        Self::classify_own_status(
            self.kind,
            self.status,
            self.activity,
            self.recovery_owner_reason.is_some(),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceStructureFact {
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
    NodeCommandPrepared {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        timestamp: f64,
    },
    NodeArtifactProduced {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        kind: WorkspaceNodeKind,
        status: WorkspaceNodeStatus,
        activity: Option<AgentSessionActivity>,
        completion_signals: NodeCompletionSignalState,
        recovery_owner_reason: Option<&str>,
    ) -> WorkspaceTreeNode {
        WorkspaceTreeNode {
            id: "node".to_string(),
            parent_id: Some("workflow".to_string()),
            sibling_order: 0,
            kind,
            title: "node".to_string(),
            status,
            status_classification: WorkspaceNodeStatusClassification::Idle,
            activity,
            error_reason: None,
            updated_at_bits: 1.0_f64.to_bits(),
            execution_id: Some("workflow".to_string()),
            node_execution_id: Some("node-execution".to_string()),
            node_name: Some("node".to_string()),
            attempt: Some(1),
            retry_predecessor_id: None,
            past_attempt_ids: Vec::new(),
            is_retry_history: false,
            completion_signals,
            has_artifact: false,
            session_id: None,
            can_approve: false,
            can_retry: false,
            can_close: false,
            can_stop: false,
            can_resume: false,
            resume_eligible: false,
            recovery_owner_reason: recovery_owner_reason.map(str::to_string),
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        }
    }

    #[test]
    fn test_詳細状態分類_優先順位と境界の組み合わせを4分類へ写像する() {
        // Given
        let cases = [
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Running,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Active,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Running,
                NodeCompletionSignalState::StopReceived,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Active,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Waiting,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Active,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Waiting,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::AwaitingAnswer),
                None,
                WorkspaceNodeStatusClassification::Attention,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Waiting,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::AwaitingInstruction),
                None,
                WorkspaceNodeStatusClassification::Attention,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Running,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::AwaitingAnswer),
                None,
                WorkspaceNodeStatusClassification::Attention,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Running,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::AwaitingInstruction),
                None,
                WorkspaceNodeStatusClassification::Attention,
            ),
            (
                WorkspaceNodeKind::WorkflowCommand,
                WorkspaceNodeStatus::Running,
                NodeCompletionSignalState::StopReceived,
                None,
                None,
                WorkspaceNodeStatusClassification::Active,
            ),
            (
                WorkspaceNodeKind::WorkflowCommand,
                WorkspaceNodeStatus::Waiting,
                NodeCompletionSignalState::Pending,
                None,
                None,
                WorkspaceNodeStatusClassification::Attention,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Failed,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Failure,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Paused,
                NodeCompletionSignalState::StopReceived,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Idle,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Completed,
                NodeCompletionSignalState::Ready,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Idle,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Aborted,
                NodeCompletionSignalState::Pending,
                Some(AgentSessionActivity::Working),
                None,
                WorkspaceNodeStatusClassification::Idle,
            ),
            (
                WorkspaceNodeKind::WorkflowSession,
                WorkspaceNodeStatus::Paused,
                NodeCompletionSignalState::StopReceived,
                Some(AgentSessionActivity::Working),
                Some("recovery fence"),
                WorkspaceNodeStatusClassification::Failure,
            ),
        ];

        // When / Then
        for (kind, status, signals, activity, recovery_reason, expected) in cases {
            assert_eq!(
                node(kind, status, activity, signals, recovery_reason).own_status_classification(),
                expected
            );
        }
    }

    #[test]
    fn test_状態分類の公開値_4分類だけを返す() {
        // Given
        let cases = [
            (WorkspaceNodeStatusClassification::Active, "active"),
            (WorkspaceNodeStatusClassification::Attention, "attention"),
            (WorkspaceNodeStatusClassification::Failure, "failure"),
            (WorkspaceNodeStatusClassification::Idle, "idle"),
        ];

        // When / Then
        for (classification, expected) in cases {
            assert_eq!(classification.as_public_str(), expected);
        }
    }
}
