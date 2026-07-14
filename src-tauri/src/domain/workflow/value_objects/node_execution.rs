use super::{NodeKindName, TokenUsage, WorkflowStepFailureKind};

/// fanout child execution が属する親 fanout と、宣言順上の位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanoutParentRef {
    pub parent_node: String,
    pub parent_attempt: u32,
    pub item_index: Option<usize>,
    pub child_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeExecutionStatus {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

impl NodeExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeExecutionFailure {
    pub reason: String,
    pub kind: WorkflowStepFailureKind,
}

/// event replay から構築する node 実行 1 回分の read model。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeExecution {
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub status: NodeExecutionStatus,
    pub session_id: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<NodeExecutionFailure>,
    pub fanout_parent: Option<FanoutParentRef>,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_execution_status_reports_active_states() {
        assert!(NodeExecutionStatus::Running.is_active());
        assert!(NodeExecutionStatus::WaitingApproval.is_active());
        assert!(!NodeExecutionStatus::Succeeded.is_active());
        assert_eq!(NodeExecutionStatus::Succeeded.as_str(), "succeeded");
    }
}
