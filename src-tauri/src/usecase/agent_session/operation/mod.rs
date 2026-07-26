//! Caller operation identity contracts for agent-session commands
//! backed by the permanent local event authority.
//!
//! This module owns the send-acceptance contract, the built-in caller
//! attempt journal, and the session lifecycle command contract on top of
//! the `LocalEventTransactionRepository` domain port. It never talks to
//! SQLite, files, Tauri, or WebSocket directly; adapters provide the
//! binding authority and runtime gates through the ports in `ports.rs`.

pub(crate) mod binding;
pub(crate) mod caller_journal;
pub(crate) mod identity;
pub(crate) mod lifecycle;
pub(crate) mod permission;
pub(crate) mod ports;
pub(crate) mod record;
pub(crate) mod recovery;
pub(crate) mod send;
pub(crate) mod stop;

#[cfg(test)]
mod acceptance_recovery_tests;
#[cfg(test)]
mod tests;

/// Shared principal for operations issued by authenticated local transports.
pub(crate) const LOCAL_INSTALLATION_OPERATION_PRINCIPAL: &str = "local-app";

pub(crate) use caller_journal::{
    BoundCallerOperation, CallerAttemptJournal, CallerJournalError, PendingCallerAttempt,
    PendingCallerAttemptPage,
};
pub(crate) use identity::{constant_time_eq_32, validate_operation_identity};
pub(crate) use lifecycle::{
    SessionLifecycleAction, SessionLifecycleCommandResult, SessionLifecycleOperationError,
    SessionLifecycleOperationState, SessionLifecycleOperationUsecase, SessionLifecycleReceipt,
    SessionLifecycleRejection, SessionLifecycleRequest,
};
pub(crate) use permission::{
    AcceptedPermissionResponseOperation, GetPermissionResponseOperationError,
    PermissionResponseCommandOutcome, PermissionResponseDecisionKind,
    PermissionResponseExecutionStatus, PermissionResponseOperationError,
    PermissionResponseOperationRequest, PermissionResponseOperationUsecase,
};
pub(crate) use ports::{
    AcceptedPermissionResponseEffect, AcceptedSendEffect, AcceptedStopEffect,
    BackendRecoveryReadbackPort, BackendRecoveryReadbackRequest, LegacyProviderEstablishRecovery,
    OperationBindingAuthority, PermissionResponseGate, PermissionResponsePlan,
    RecoveryEffectExecutor, RecoveryEffectHandoff, RecoveryEffectRequest, RecoveryEffectResult,
    RecoveryOwnerBatch, RecoveryResultCanonicalizer, SendAdmissionGate, SendEffectDispatch,
    SendPlan, SendRecoveryReadbackKind, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
    SessionCloseRecoveryReadbackPort, SessionCloseRecoveryReadbackRequest, SessionLifecycleEffect,
    SessionLifecycleGate, SessionLifecycleSnapshot, SessionLifecycleState,
    StableRecoveryEffectIdentity, StopAdmissionGate, StopEffectObservation,
    StopRecoveryReadbackPort, StopRecoveryReadbackRequest, StopTargetSnapshot,
    TerminalParticipants,
};
pub(crate) use recovery::{
    decode_recovery_completed_result, derive_recovery_action_id, PendingRecoveryCategory,
    PendingRecoveryEntry, PendingRecoveryKnownStatus, PendingRecoveryOwnerTarget,
    PendingRecoveryPage, PendingRecoveryQuery, PendingRecoverySnapshotQuery, RecoveryActionError,
    RecoveryActionIdentity, RecoveryActionOutcome, RecoveryActionRejection, RecoveryActionRequest,
    RecoveryActionResultOutcome, RecoveryActionStatus, RecoveryActionUsecase,
};
pub(crate) use send::{
    AcceptedSendOperation, AgentSendOperationUsecase, GetSendOperationError,
    ObligationTransitionOutcome, SendAgentMessageError, SendCommandOutcome, SendExecutionStatus,
    SendOperationRequest,
};
pub(crate) use stop::{
    StopCommandOutcome, StopOperationError, StopOperationReceipt, StopOperationRequest,
    StopOperationState, StopOperationUsecase,
};
