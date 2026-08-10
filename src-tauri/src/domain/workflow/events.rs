//! Canonical closed workflow execution event vocabulary.
//!
//! NDJSON/SQLite envelopes and public DTOs are adapter concerns. Node execution
//! lifecycle events live only in this workflow domain module and are never mixed
//! into the agent-session event stream.

use super::{
    ExecutionInterruptionReason, ExecutionOrigin, FanoutParentRef, NodeExecutionFailureKind,
    NodeKindName, TokenUsage, WorkflowDefinition,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowJsonPayload(String);

impl WorkflowJsonPayload {
    pub fn new_validated(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowContractViolation {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowDomainEvent {
    WorkflowExecutionStarted {
        execution_id: String,
        workflow_name: String,
        worktree_path: String,
        created_from: ExecutionOrigin,
        request: String,
        permission_mode: String,
        definition: WorkflowDefinition,
        timestamp: f64,
    },
    NodeExecutionStarted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        timestamp: f64,
    },
    NodeExecutionAgentBound {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    NodeExecutionSubmitReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeExecutionStopReceived {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeExecutionRetryRequested {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeExecutionPaused {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeExecutionResumed {
        execution_id: String,
        node_execution_id: String,
        timestamp: f64,
    },
    NodeExecutionCommandPrepared {
        execution_id: String,
        node_execution_id: String,
        display_command: String,
        timestamp: f64,
    },
    WorkflowArtifactProduced {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        contract: Option<String>,
        value: WorkflowJsonPayload,
        request_id: Option<String>,
        submitted_at: Option<f64>,
        timestamp: f64,
    },
    NodeExecutionCompleted {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        result_summary: Option<String>,
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    },
    NodeExecutionFailed {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
        timestamp: f64,
    },
    WorkflowApprovalRequested {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        timestamp: f64,
    },
    WorkflowApprovalResolved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        comment: Option<String>,
        timestamp: f64,
    },
    WorkflowContractViolated {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        violations: Vec<WorkflowContractViolation>,
        repair_attempt: u32,
        request_id: Option<String>,
        timestamp: f64,
    },
    NodeExecutionStallObserved {
        execution_id: String,
        node_execution_id: String,
        node_name: String,
        attempt: u32,
        session_id: String,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
        timestamp: f64,
    },
    NodeExecutionStallCleared {
        execution_id: String,
        node_execution_id: String,
        session_id: String,
        timestamp: f64,
    },
    WorkflowExecutionCompleted {
        execution_id: String,
        total_token_usage: TokenUsage,
        timestamp: f64,
    },
    WorkflowExecutionAborted {
        execution_id: String,
        aborted_node: Option<String>,
        timestamp: f64,
    },
    WorkflowExecutionInterrupted {
        execution_id: String,
        reason: ExecutionInterruptionReason,
        timestamp: f64,
    },
    WorkflowExecutionResumed {
        execution_id: String,
        resume_from_node: String,
        timestamp: f64,
    },
}

impl WorkflowDomainEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::WorkflowExecutionStarted { execution_id, .. }
            | Self::NodeExecutionStarted { execution_id, .. }
            | Self::NodeExecutionAgentBound { execution_id, .. }
            | Self::NodeExecutionSubmitReceived { execution_id, .. }
            | Self::NodeExecutionStopReceived { execution_id, .. }
            | Self::NodeExecutionRetryRequested { execution_id, .. }
            | Self::NodeExecutionPaused { execution_id, .. }
            | Self::NodeExecutionResumed { execution_id, .. }
            | Self::NodeExecutionCommandPrepared { execution_id, .. }
            | Self::WorkflowArtifactProduced { execution_id, .. }
            | Self::NodeExecutionCompleted { execution_id, .. }
            | Self::NodeExecutionFailed { execution_id, .. }
            | Self::WorkflowApprovalRequested { execution_id, .. }
            | Self::WorkflowApprovalResolved { execution_id, .. }
            | Self::WorkflowContractViolated { execution_id, .. }
            | Self::NodeExecutionStallObserved { execution_id, .. }
            | Self::NodeExecutionStallCleared { execution_id, .. }
            | Self::WorkflowExecutionCompleted { execution_id, .. }
            | Self::WorkflowExecutionAborted { execution_id, .. }
            | Self::WorkflowExecutionInterrupted { execution_id, .. }
            | Self::WorkflowExecutionResumed { execution_id, .. } => execution_id,
        }
    }
}
