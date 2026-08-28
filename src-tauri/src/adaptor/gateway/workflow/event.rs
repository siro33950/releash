//! Workflow runtime event vocabulary (in-memory engine language).
//!
//! 永続形は統一 Node 事実ログ（`fact_log`）であり、この event 列自体は
//! 保存されない。ここでは domain の型を gateway 内へ再輸出し、
//! 事実写像が使う行メタ用 accessor を提供する。

pub use crate::domain::workflow::WorkflowEvent;

impl WorkflowEvent {
    pub fn execution_id(&self) -> &str {
        match self {
            Self::ExecutionStarted { execution_id, .. }
            | Self::NodeStarted { execution_id, .. }
            | Self::SessionAttached { execution_id, .. }
            | Self::NodeSubmitReceived { execution_id, .. }
            | Self::NodeStopReceived { execution_id, .. }
            | Self::NodeRetryRequested { execution_id, .. }
            | Self::NodePaused { execution_id, .. }
            | Self::NodeResumed { execution_id, .. }
            | Self::NodeProcessExitObserved { execution_id, .. }
            | Self::CommandSpawned { execution_id, .. }
            | Self::ArtifactProduced { execution_id, .. }
            | Self::NodeCompleted { execution_id, .. }
            | Self::NodeFailed { execution_id, .. }
            | Self::ApprovalRequested { execution_id, .. }
            | Self::ApprovalResolved { execution_id, .. }
            | Self::ContractViolated { execution_id, .. }
            | Self::StallObserved { execution_id, .. }
            | Self::StallCleared { execution_id, .. }
            | Self::ExecutionCompleted { execution_id, .. }
            | Self::ExecutionAborted { execution_id, .. }
            | Self::ExecutionInterrupted { execution_id, .. }
            | Self::ExecutionResumed { execution_id, .. } => execution_id,
        }
    }

    pub fn timestamp(&self) -> f64 {
        match self {
            Self::ExecutionStarted { timestamp, .. }
            | Self::NodeStarted { timestamp, .. }
            | Self::SessionAttached { timestamp, .. }
            | Self::NodeSubmitReceived { timestamp, .. }
            | Self::NodeStopReceived { timestamp, .. }
            | Self::NodeRetryRequested { timestamp, .. }
            | Self::NodePaused { timestamp, .. }
            | Self::NodeResumed { timestamp, .. }
            | Self::NodeProcessExitObserved { timestamp, .. }
            | Self::CommandSpawned { timestamp, .. }
            | Self::ArtifactProduced { timestamp, .. }
            | Self::NodeCompleted { timestamp, .. }
            | Self::NodeFailed { timestamp, .. }
            | Self::ApprovalRequested { timestamp, .. }
            | Self::ApprovalResolved { timestamp, .. }
            | Self::ContractViolated { timestamp, .. }
            | Self::StallObserved { timestamp, .. }
            | Self::StallCleared { timestamp, .. }
            | Self::ExecutionCompleted { timestamp, .. }
            | Self::ExecutionAborted { timestamp, .. }
            | Self::ExecutionInterrupted { timestamp, .. }
            | Self::ExecutionResumed { timestamp, .. } => *timestamp,
        }
    }
}
