//! Permanent local event store domain.
//!
//! Owns the closed vocabulary of the single mutation authority spanning
//! agent sessions and workflows: store identities, the atomic batch, the
//! closed state-mutation family, the closed query sum, and the
//! `LocalEventTransactionRepository` port. No serde, rusqlite, filesystem,
//! Tauri, or WebSocket dependency is allowed here.

pub mod batch;
pub mod events;
pub mod failure;
pub mod identifiers;
pub mod mutation;
pub mod operation_identity;
pub mod operation_record;
pub mod query;
pub mod record;
#[allow(clippy::module_inception)]
pub mod repository;

pub use batch::{
    CommitBatchError, CommitBatchResult, CommitOperationKind, CommitResolution, CommittedBatch,
    CommittedStreamHead, IdempotencyBinding, LocalAtomicBatch,
};
pub use events::{
    ApplicationDomainEvent, ApplicationShutdownPhase, CommittedDomainEvent, DomainEventPage,
    LoadStreamRequest, LoadedDomainEvent, LocalDomainEvent, QuitIntent, UncommittedDomainEvent,
};
pub use failure::{SafeEffectObservation, SafeOperationFailure, SessionOperationFailureKind};
pub use identifiers::{
    CommitIdentity, EventId, ExpectedStreamHead, GlobalSequence, Revision, StreamId,
    StreamSequence, StreamVersion,
};
pub use mutation::{
    CallerAttemptMutation, CallerAttemptResolution, CallerOperationKey, LocalStateMutation,
    MessageProjectionMutation, ObligationMutation, OperationBindingMutation, OperationKind,
    OperationRecordMutation, PendingIndexEntry, PendingPartition, RecoveryActionMutation,
    RevisionGuard, SessionProjectionMutation, SessionProjectionRemovalMutation,
    ShutdownDetailsCompactionMutation, ShutdownDetailsState, ShutdownLatestPointerMutation,
    ShutdownPlanKey, ShutdownPlanMutation, ShutdownRecoverySnapshotMutation,
    ShutdownTargetMutation, StopResolutionKind, StopResolutionMutation, TerminalRecordMutation,
    WorkflowExecutionNodeProjectionMutation, WorkflowExecutionProjectionMutation,
};
#[cfg(test)]
pub use operation_identity::session_projection_rollback_identity;
pub use operation_identity::{
    agent_atomic_event_payload_identity, agent_event_payload_identity,
    backend_recovery_readback_participant_identity, constant_time_eq_32,
    decide_workflow_turn_completion_identity, decide_workflow_turn_completion_settlement,
    hash_event_projection_meta_patch, hash_terminal_message_projection_patch, hex_lower,
    runtime_terminal_identity, session_closed_terminal_identity_material,
    session_projection_binding_identity, sha256, validate_operation_identity,
    validate_pending_workflow_turn_completion, validate_workflow_turn_completion_notification,
    validate_workflow_turn_completion_terminal, workflow_turn_completion_consume_commit_identity,
    workflow_turn_completion_ordered_key_prefix, DurableIdentityBuilder,
    EventProjectionMetaIdentityFacts, PendingWorkflowTurnCompletionRejection,
    RecoveryPublicationMessageIdentityFacts, ValidatedPendingWorkflowTurnCompletion,
    WorkflowTurnCompletionIdentityFacts, WorkflowTurnCompletionSettlementDecision,
    WorkflowTurnCompletionSettlementFacts, WorkflowTurnCompletionSettlementRejection,
};
pub use operation_record::{
    validate_operation_record, validate_stop_resolution, validate_terminal_record,
};
pub use query::{
    AgentSessionLifecycleSnapshotView, CallerAttemptView, CanonicalRuntimeOwnerView,
    LocalEventQuery, LocalEventQueryError, LocalEventQueryResult, MessageProjectionPageEntryView,
    MessageProjectionPageView, MessageProjectionView, ObligationView, OperationBindingSummaryView,
    OperationBindingView, OperationRecordView, PendingIndexEntryView, PendingObligationView,
    PendingRecoveryPageView, PendingRecoverySnapshotPageView, ProviderAgentSessionOriginKind,
    ProviderAgentSessionProjectionPageView, QueryCursor, RecoveryActionView,
    SessionProjectionOwnerState, SessionProjectionView, ShutdownPlanPageView, ShutdownPlanView,
    ShutdownSnapshotEntryView, ShutdownTargetView, StopResolutionView, TerminalRecordView,
};
pub use record::{
    AgentContentBlobRecord, AgentContextCarryStateRecord, AgentContextEpochRecord,
    AgentContextSourceRecord, AgentMessageActivityRecord, AgentMessageProjectionRecord,
    AgentMessageRoleRecord, AgentPendingRecoveryMessageRecord, AgentQueuedSendRecord,
    AgentRecoveryPublicationClassificationRecord, AgentRecoveryPublicationListRecord,
    AgentRecoveryPublicationSnapshotRecord, AgentRecoveryPublicationWorkflowOwnerRecord,
    AgentSessionMetadataRecord, AgentSessionNoticeOperationRecord, AgentSessionProjectionRecord,
    AgentSessionStateRecord, AgentSessionSummaryRecord, AgentTerminalKind,
    AgentTurnInterruptionRecord, AgentTurnTerminalResultRecord,
    AuthoritativeEffectObservationRecord, BackendSessionRecoveryObligationRecord,
    FeedbackActionRecord, MessageProjectionRecord, ObligationRecord,
    ObligationRecoveryActionRecord, ObligationStateRecord, OperationReceiptRecord,
    OperationStatusRecord, OperationStatusValue, PermissionDecisionRecord,
    ProviderAgentSessionLifecycleRecord, ProviderAgentSessionOriginRecord,
    ProviderAgentSessionProjectionRecord, ProviderAgentSessionProviderRecord,
    ProviderHookHealthProjectionRecord, ProviderSessionOwnershipProjectionRecord,
    RecordAuthentication, RecoveryActionResultRecord, RecoveryAttemptRecord,
    RecoveryPublicationMessageKindRecord, RecoveryPublicationMessageRecord,
    RecoveryPublicationObligationRecord, RecoveryResourceViewRecord, RecoveryResultOutcomeRecord,
    RecoveryResultRecord, SendObligationDispositionRecord, SendObligationKindRecord,
    SessionLifecycleRecordAction, SessionProjectionRecord, ShutdownOutcomeRecord,
    ShutdownPlanRecord, ShutdownTargetKindRecord, ShutdownTargetRecord,
    ShutdownTargetRecoveryRecord, ShutdownTargetStateRecord, TerminalInterruptReasonRecord,
    TerminalResultRecord, WorkflowExecutionMetadataRecord, WorkflowExecutionProjectionRecord,
    WorkflowObligationRetirementReason, WorkflowObligationTerminalOutcome,
    WorkflowTurnCompletionObligationRecord, WorkflowTurnFailureSignalRecord,
    WorkflowWorktreeOwnerRecord,
};
pub use repository::{LocalEventSignal, LocalEventSubscription, LocalEventTransactionRepository};
