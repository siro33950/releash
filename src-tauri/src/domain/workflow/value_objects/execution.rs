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

    /// 実行を再開できない最終状態かどうか。
    ///
    /// `Interrupted` は process / session が動いていないが、event log から再開できる
    /// checkpoint なので finished ではない。
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Aborted)
    }

    pub fn is_terminal(self) -> bool {
        self.is_finished()
    }

    pub fn is_resumable(self) -> bool {
        self == Self::Interrupted
    }

    pub fn can_stop(self) -> bool {
        self.is_active()
    }

    pub fn can_resume(self) -> bool {
        self.is_resumable()
    }

    pub fn can_abort(self) -> bool {
        self.is_active() || self.is_resumable()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionInterruptionReason {
    Crash,
    Stale,
    Stop,
    Orphan,
}

impl ExecutionInterruptionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Stale => "stale",
            Self::Stop => "stop",
            Self::Orphan => "orphan",
        }
    }

    pub fn from_reason(reason: &str) -> Option<Self> {
        match reason {
            "crash" => Some(Self::Crash),
            "stale" => Some(Self::Stale),
            "stop" => Some(Self::Stop),
            "orphan" => Some(Self::Orphan),
            _ => None,
        }
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
    pub interruption_reason: Option<ExecutionInterruptionReason>,
    pub resume_from_node: Option<String>,
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
        assert!(!ExecutionStatus::Interrupted.is_finished());
        assert!(ExecutionStatus::Interrupted.is_resumable());
        assert!(ExecutionStatus::Completed.is_finished());
        assert_eq!(ExecutionStatus::Interrupted.as_str(), "interrupted");
    }

    #[test]
    fn execution_command_permission_matrix_matches_the_typed_contract() {
        let cases = [
            (ExecutionStatus::Running, true, false, true),
            (ExecutionStatus::WaitingApproval, true, false, true),
            (ExecutionStatus::Interrupted, false, true, true),
            (ExecutionStatus::Completed, false, false, false),
            (ExecutionStatus::Failed, false, false, false),
            (ExecutionStatus::Aborted, false, false, false),
        ];

        for (status, can_stop, can_resume, can_abort) in cases {
            assert_eq!(status.can_stop(), can_stop, "stop from {status:?}");
            assert_eq!(status.can_resume(), can_resume, "resume from {status:?}");
            assert_eq!(status.can_abort(), can_abort, "abort from {status:?}");
        }
    }

    #[test]
    fn interruption_reason_uses_canonical_event_vocabulary() {
        for reason in [
            ExecutionInterruptionReason::Crash,
            ExecutionInterruptionReason::Stale,
            ExecutionInterruptionReason::Stop,
            ExecutionInterruptionReason::Orphan,
        ] {
            assert_eq!(
                ExecutionInterruptionReason::from_reason(reason.as_str()),
                Some(reason)
            );
        }
        assert_eq!(ExecutionInterruptionReason::from_reason("legacy"), None);
    }
}
