use super::{NodeExecution, TokenUsage};
use crate::domain::workflow::error::WorkflowError;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Running,
    Completed,
    Aborted,
}

impl ExecutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Aborted => "aborted",
        }
    }

    pub fn is_active(self) -> bool {
        self == Self::Running
    }

    /// 実行を再開できない最終状態かどうか。
    ///
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Aborted)
    }

    pub fn can_abort(self) -> bool {
        self.is_active()
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
pub struct ExecutionTree {
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

impl ExecutionTree {
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
#[path = "execution_test.rs"]
mod execution_tests;
