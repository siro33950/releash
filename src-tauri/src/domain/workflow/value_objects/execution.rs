use super::{NodeExecution, TokenUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
    Interrupted,
}

impl ExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Aborted | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionOrigin {
    DesktopUi,
    Cli,
    Agent,
    Api,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub node_name: String,
    pub contract: Option<String>,
    pub value: serde_json::Value,
    pub produced_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fanout {
    pub parent: NodeExecution,
    pub children: Vec<NodeExecution>,
    pub artifact: Option<Artifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTarget {
    pub node_execution_id: String,
    pub node_name: String,
    pub session_id: Option<String>,
}

/// Event replay から構築される backend-owned workflow read model。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecution {
    pub id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub current_node: Option<String>,
    pub created_from: ExecutionOrigin,
    pub worktree_path: String,
    pub started_at: f64,
    pub updated_at: f64,
    pub completed_at: Option<f64>,
    pub error_reason: Option<String>,
    pub total_token_usage: TokenUsage,
    pub node_executions: Vec<NodeExecution>,
    pub artifacts: Vec<Artifact>,
    pub fanouts: Vec<Fanout>,
    pub approval_target: Option<ApprovalTarget>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_status_reports_active_states() {
        assert!(ExecutionStatus::Running.is_active());
        assert!(ExecutionStatus::WaitingApproval.is_active());
        assert!(!ExecutionStatus::Completed.is_active());
        assert_eq!(ExecutionStatus::Interrupted.as_str(), "interrupted");
    }
}
