use crate::domain::workflow::{
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus, TokenUsage as WorkflowTokenUsage,
};

use super::{
    OperationKind, QuitIntent, RecoveryActionKind, RecoveryResultClassification,
    SafeOperationFailure, ShutdownPlanKey,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowExecutionProjectionRecord {
    Present(WorkflowExecutionMetadataRecord),
    Deleted { execution_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionProviderRecord {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSessionOriginRecord {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSessionLifecycleRecord {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionProjectionRecord {
    pub id: String,
    pub workspace_identity: String,
    pub worktree_path: String,
    pub provider: AgentSessionProviderRecord,
    pub origin: AgentSessionOriginRecord,
    pub lifecycle: AgentSessionLifecycleRecord,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
    pub initial_instruction_admitted: bool,
    pub last_exit_abnormal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionOwnershipProjectionRecord {
    pub provider: AgentSessionProviderRecord,
    pub provider_session_id: String,
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookHealthProjectionRecord {
    pub provider: AgentSessionProviderRecord,
    pub latest_launch_id: String,
    pub latest_launch_session_started: bool,
    pub warning_launch_id: Option<String>,
    pub warning_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowWorktreeOwnerRecord {
    pub worktree_path: String,
    pub execution_id: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionProjectionRecord {
    AgentSession(AgentSessionProjectionRecord),
    ProviderSessionOwnership(ProviderSessionOwnershipProjectionRecord),
    ProviderHookHealth(ProviderHookHealthProjectionRecord),
    WorkflowExecution(WorkflowExecutionProjectionRecord),
    WorkflowWorktreeOwner(WorkflowWorktreeOwnerRecord),
}

impl SessionProjectionRecord {
    pub(crate) fn semantic_bytes(&self) -> usize {
        fn optional(value: &Option<String>) -> usize {
            value.as_ref().map_or(0, String::len)
        }
        match self {
            Self::AgentSession(value) => {
                256 + value.id.len()
                    + value.workspace_identity.len()
                    + value.worktree_path.len()
                    + optional(&value.provider_session_id)
                    + optional(&value.transcript_ref)
            }
            Self::ProviderSessionOwnership(value) => {
                128 + value.provider_session_id.len() + optional(&value.agent_session_id)
            }
            Self::ProviderHookHealth(value) => {
                96 + value.latest_launch_id.len()
                    + optional(&value.warning_launch_id)
                    + optional(&value.warning_reason)
            }
            Self::WorkflowExecution(WorkflowExecutionProjectionRecord::Present(value)) => {
                256 + value.execution_id.len()
                    + value.workflow_name.len()
                    + value.worktree_path.len()
                    + optional(&value.current_node)
                    + optional(&value.error_reason)
                    + optional(&value.resume_from_node)
            }
            Self::WorkflowExecution(WorkflowExecutionProjectionRecord::Deleted {
                execution_id,
            }) => 64 + execution_id.len(),
            Self::WorkflowWorktreeOwner(value) => {
                96 + value.worktree_path.len() + value.execution_id.len()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationReceiptRecord {
    ApplicationQuit {
        operation_id: String,
        plan: ShutdownPlanKey,
        intent: QuitIntent,
        t0_ms: i64,
        deadline_ms: i64,
        binding_hmac: [u8; 32],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationStatusValue {
    Preparing,
    Activated,
    Completed,
    OutcomeUnknown {
        operation_id: String,
        plan: ShutdownPlanKey,
        activation_commit_id: String,
    },
    FailedBeforeActivation {
        failure: SafeOperationFailure,
    },
    ReconciliationRequired {
        failure: SafeOperationFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationStatusRecord {
    pub kind: OperationKind,
    pub value: OperationStatusValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationStateRecord {
    Prepared,
    Pending,
    EffectReserved,
    Running,
    WaitingApproval,
    OutcomeUnknown,
    ReconciliationRequired,
    Failed,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionMetadataRecord {
    pub execution_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub worktree_path: String,
    pub current_node: Option<String>,
    pub created_from: ExecutionOrigin,
    pub started_at_bits: u64,
    pub updated_at_bits: u64,
    pub completed_at_bits: Option<u64>,
    pub error_reason: Option<String>,
    pub interruption_reason: Option<ExecutionInterruptionReason>,
    pub resume_from_node: Option<String>,
    pub total_token_usage: WorkflowTokenUsage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObligationRecord {
    WorkflowShutdown {
        operation_id: String,
        effect_identity: String,
        owner_revision: i64,
        execution_id: String,
        state: ObligationStateRecord,
    },
    WorkflowExecution {
        execution: WorkflowExecutionMetadataRecord,
    },
}

impl ObligationRecord {
    pub(crate) fn blocks_effect_admission(&self) -> bool {
        matches!(
            self,
            Self::WorkflowShutdown { state, .. }
                if !matches!(state, ObligationStateRecord::Completed | ObligationStateRecord::Cancelled)
        )
    }

    pub(crate) fn unresolved_recovery_original_identity(
        &self,
        obligation_id: &str,
    ) -> Option<String> {
        if !self.blocks_effect_admission() {
            return None;
        }
        match self {
            Self::WorkflowShutdown {
                effect_identity,
                execution_id,
                ..
            } => Some(
                [
                    effect_identity.as_str(),
                    execution_id.as_str(),
                    obligation_id,
                ]
                .into_iter()
                .find(|value| !value.is_empty() && value.len() <= 512)
                .unwrap_or(obligation_id)
                .to_string(),
            ),
            Self::WorkflowExecution { .. } => None,
        }
    }

    pub(crate) fn write_canonical_identity_v1(
        &self,
        bytes: &mut Vec<u8>,
    ) -> Result<(), &'static str> {
        fn text(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        match self {
            Self::WorkflowShutdown {
                operation_id,
                effect_identity,
                owner_revision,
                execution_id,
                state,
            } => {
                text(bytes, "workflow_shutdown");
                for value in [operation_id, effect_identity, execution_id] {
                    text(bytes, value);
                }
                bytes.extend_from_slice(&owner_revision.to_be_bytes());
                bytes.push(*state as u8);
            }
            Self::WorkflowExecution { execution } => {
                text(bytes, "workflow_execution");
                text(bytes, &execution.execution_id);
                text(bytes, &execution.workflow_name);
                text(bytes, &execution.worktree_path);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAttemptRecord {
    ShutdownTarget {
        resource_ref: String,
        plan: ShutdownPlanKey,
        ordinal: i64,
        target_key: String,
        origin_revision: u64,
        action: RecoveryActionKind,
        effect_identity_sha256: [u8; 32],
        intent: QuitIntent,
        state: ObligationStateRecord,
        failure: Option<SafeOperationFailure>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryResultOutcomeRecord {
    Pending,
    Terminal,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResourceViewRecord {
    ShutdownTarget {
        plan: ShutdownPlanKey,
        ordinal: i64,
        target_id: String,
        state: ShutdownTargetStateRecord,
    },
    SafeSummary(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionResultRecord {
    pub outcome: RecoveryResultOutcomeRecord,
    pub classification: RecoveryResultClassification,
    pub resource_revision: u64,
    pub canonical_result_sha256: [u8; 32],
    pub resource_view: RecoveryResourceViewRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryResultRecord {
    Action(RecoveryActionResultRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownTargetKindRecord {
    WorkflowExecution,
    WorkflowNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownTargetStateRecord {
    Prepared,
    EffectReserved,
    Completed,
    Failed,
    ReconciliationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcomeRecord {
    Completed,
    AbortedBeforeActivation,
    ReconciliationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownPlanRecord {
    pub operation_id: String,
    pub intent: QuitIntent,
    pub t0_ms: i64,
    pub preparation_cutoff_ms: Option<i64>,
    pub deadline_ms: i64,
    pub target_count: Option<u64>,
    pub prepared_count: Option<u64>,
    pub effect_reserved_count: Option<u64>,
    pub terminal_count: Option<u64>,
    pub completed_count: Option<u64>,
    pub unresolved_count: Option<u64>,
    pub recovery_snapshot_count: Option<u64>,
    pub recovery_snapshot_id: Option<String>,
    pub process_instance_id: String,
    pub outcome: Option<ShutdownOutcomeRecord>,
    pub failure: Option<SafeOperationFailure>,
    pub shutdown_effect_count: Option<u64>,
    pub admission_open: Option<bool>,
    pub retry_quit_same_boot: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownTargetRecoveryRecord {
    pub action_id: String,
    pub origin_revision: u64,
    pub action: RecoveryActionKind,
    pub state: ObligationStateRecord,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShutdownTargetRecord {
    Target {
        target_id: String,
        kind: ShutdownTargetKindRecord,
        state: ShutdownTargetStateRecord,
        effect_identity: String,
        owner_operation_id: Option<String>,
        failure: Option<SafeOperationFailure>,
        recovery_action: Option<ShutdownTargetRecoveryRecord>,
    },
    RecoverySnapshot {
        obligation_id: String,
        ordered_key: String,
        owner: String,
        revision: u64,
        record: Box<ObligationRecord>,
    },
}
