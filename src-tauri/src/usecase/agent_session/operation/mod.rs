//! Caller operation identity contracts for agent-session commands
//! backed by the permanent local event authority.
//!
//! This module owns the send-acceptance contract, the built-in caller
//! attempt journal, and the session lifecycle command contract on top of
//! the `LocalEventTransactionRepository` domain port. It never talks to
//! SQLite, files, Tauri, or WebSocket directly; adapters provide the
//! binding authority, projection preparation, and post-commit effects through
//! the ports in `ports.rs`.

pub(crate) mod binding;
pub(crate) mod caller_journal;
pub(crate) mod identity;
pub(crate) mod lifecycle;
pub(crate) mod permission;
pub(crate) mod ports;
pub(crate) mod record;
pub(crate) mod recovery;
pub(crate) mod runtime_adapter;
pub(crate) mod runtime_drivers;
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
    OperationBindingAuthority, PermissionResponseEffectPort, RecoveryEffectExecutor,
    RecoveryEffectHandoff, RecoveryEffectRequest, RecoveryEffectResult, RecoveryOwnerBatch,
    RecoveryResultCanonicalizer, SendAcceptancePort, SendEffectDispatch, SendPlan,
    SendRecoveryReadbackKind, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
    SessionCloseRecoveryReadbackPort, SessionCloseRecoveryReadbackRequest, SessionLifecycleEffect,
    SessionLifecycleEffectPort, StableRecoveryEffectIdentity, StopEffectObservation,
    StopEffectPort, StopRecoveryReadbackPort, StopRecoveryReadbackRequest, TerminalParticipants,
};
#[cfg(test)]
pub(crate) use ports::{SessionLifecycleSnapshot, SessionLifecycleState, StopTargetSnapshot};
pub(crate) use recovery::{
    decode_recovery_completed_result, derive_recovery_action_id, PendingRecoveryCategory,
    PendingRecoveryEntry, PendingRecoveryKnownStatus, PendingRecoveryOwnerTarget,
    PendingRecoveryPage, PendingRecoveryQuery, PendingRecoverySnapshotQuery, RecoveryActionError,
    RecoveryActionIdentity, RecoveryActionOutcome, RecoveryActionRejection, RecoveryActionRequest,
    RecoveryActionResultOutcome, RecoveryActionStatus, RecoveryActionUsecase,
};
pub(crate) use runtime_adapter::{
    CanonicalSendCommandCodec, DecodedSendCommand, DecodedSendTarget,
};
#[cfg(test)]
pub(crate) use runtime_drivers::bind_runtime_durable_workflow_send_driver;
pub(crate) use runtime_drivers::{
    bind_runtime_durable_stop_driver, bind_runtime_terminal_operation_participant_provider,
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
