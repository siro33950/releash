use super::{Artifact, NodeExecutionFailureKind, NodeKindName, TokenUsage};

/// fanout child execution が属する親 fanout と、宣言順上の位置。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanoutParentRef {
    pub parent_node: String,
    pub parent_attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
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
    pub kind: NodeExecutionFailureKind,
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
    pub display_command: Option<String>,
    pub result_summary: Option<String>,
    pub artifact: Option<Artifact>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<NodeExecutionFailure>,
    pub fanout_parent: Option<FanoutParentRef>,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

impl NodeExecution {
    pub fn replay_completed(
        &mut self,
        result_summary: Option<String>,
        token_usage: Option<TokenUsage>,
        completed_at: f64,
    ) {
        self.status = NodeExecutionStatus::Succeeded;
        self.result_summary = result_summary;
        self.token_usage = token_usage;
        self.failure = None;
        self.completed_at = Some(completed_at);
    }

    pub fn replay_failed(&mut self, failure: NodeExecutionFailure, completed_at: f64) {
        self.status = NodeExecutionStatus::Failed;
        self.failure = Some(failure);
        self.completed_at = Some(completed_at);
    }

    pub fn replay_approval_requested(&mut self) {
        self.status = NodeExecutionStatus::WaitingApproval;
    }

    pub fn replay_approval_resolved(&mut self) {
        self.status = NodeExecutionStatus::Running;
    }
}

impl NodeExecution {
    pub fn record_artifact(&mut self, artifact: Artifact) {
        self.artifact = Some(artifact);
    }
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
