use super::{NodeExecution, TokenUsage};
use crate::domain::workflow::error::WorkflowError;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Running,
    #[cfg(test)]
    WaitingApproval,
    Completed,
    Aborted,
    #[cfg(test)]
    Interrupted,
}

impl ExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            #[cfg(test)]
            Self::WaitingApproval => "waiting_approval",
            Self::Completed => "completed",
            Self::Aborted => "aborted",
            #[cfg(test)]
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_active(self) -> bool {
        match self {
            Self::Running => true,
            #[cfg(test)]
            Self::WaitingApproval => true,
            _ => false,
        }
    }

    /// 実行を再開できない最終状態かどうか。
    ///
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Aborted)
    }

    pub fn is_terminal(self) -> bool {
        self.is_finished()
    }

    pub fn is_resumable(self) -> bool {
        #[cfg(test)]
        if self == Self::Interrupted {
            return true;
        }
        false
    }

    pub fn can_stop(self) -> bool {
        self.is_active()
    }

    pub fn can_resume(self) -> bool {
        self == Self::Running || self.is_resumable()
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

    #[cfg(test)]
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

impl ExecutionOrigin {
    pub fn as_public_value(self) -> &'static str {
        match self {
            Self::DesktopUi => "desktop_ui",
            Self::Cli => "cli",
            Self::Agent => "agent",
            Self::Api => "api",
        }
    }

    pub fn from_public_value(value: &str) -> Result<Self, WorkflowError> {
        match value {
            "desktop_ui" | "desktop-ui" => Ok(Self::DesktopUi),
            "cli" => Ok(Self::Cli),
            "agent" => Ok(Self::Agent),
            "api" => Ok(Self::Api),
            other => Err(WorkflowError::validation(format!(
                "unknown created_from value: {other}"
            ))),
        }
    }
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

impl WorkflowExecution {
    pub fn retryable_node_execution_ids(&self) -> HashSet<String> {
        self.node_executions
            .iter()
            .filter(|node| node.can_retry())
            .filter(|node| {
                self.node_executions.iter().all(|candidate| {
                    candidate.node_name != node.node_name
                        || candidate.parent != node.parent
                        || candidate.attempt <= node.attempt
                })
            })
            .map(|node| node.id.clone())
            .collect()
    }
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
    fn execution_origin_owns_the_public_vocabulary_and_rejects_unknown_values() {
        for (value, expected) in [
            ("desktop_ui", ExecutionOrigin::DesktopUi),
            ("desktop-ui", ExecutionOrigin::DesktopUi),
            ("cli", ExecutionOrigin::Cli),
            ("agent", ExecutionOrigin::Agent),
            ("api", ExecutionOrigin::Api),
        ] {
            assert_eq!(ExecutionOrigin::from_public_value(value).unwrap(), expected);
            assert_eq!(
                ExecutionOrigin::from_public_value(expected.as_public_value()).unwrap(),
                expected
            );
        }
        assert!(ExecutionOrigin::from_public_value("remote").is_err());
    }

    #[test]
    fn execution_command_permission_matrix_matches_the_typed_contract() {
        let cases = [
            (ExecutionStatus::Running, true, true, true),
            (ExecutionStatus::Completed, false, false, false),
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

    #[test]
    fn retryable_node_ids_only_include_the_latest_current_attempt() {
        let node = |id: &str, node_name: &str, attempt: u32| NodeExecution {
            recovery_reason: None,
            id: id.to_string(),
            execution_id: "execution-1".to_string(),
            node_name: node_name.to_string(),
            kind: super::super::NodeKindName::Session,
            attempt,
            status: super::super::NodeExecutionStatus::Failed,
            session_id: None,
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure: None,
            parent: None,
            completion_signals: super::super::NodeCompletionSignalState::Pending,
            started_at: 1.0,
            completed_at: Some(2.0),
        };
        let execution = WorkflowExecution {
            id: "execution-1".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::Cli,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
            node_executions: vec![
                node("review-1", "review", 1),
                node("review-2", "review", 2),
                node("other-1", "other", 1),
            ],
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        };

        assert_eq!(
            execution.retryable_node_execution_ids(),
            std::collections::HashSet::from(["review-2".to_string(), "other-1".to_string()])
        );
    }
}
