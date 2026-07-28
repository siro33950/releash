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
};
pub use operation_record::{
    validate_operation_record, validate_stop_resolution, validate_terminal_record,
};
pub use query::{
    CallerAttemptView, CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, MessageProjectionPageEntryView, MessageProjectionPageView,
    MessageProjectionView, ObligationView, OperationBindingSummaryView, OperationBindingView,
    OperationRecordView, PendingIndexEntryView, PendingObligationView, PendingRecoveryPageView,
    PendingRecoverySnapshotPageView, QueryCursor, RecoveryActionView, SessionProjectionOwnerState,
    SessionProjectionView, ShutdownPlanPageView, ShutdownPlanView, ShutdownSnapshotEntryView,
    ShutdownTargetView, StopResolutionView, TerminalRecordView,
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
    OperationStatusRecord, OperationStatusValue, PermissionDecisionRecord, RecordAuthentication,
    RecoveryActionResultRecord, RecoveryAttemptRecord, RecoveryPublicationMessageKindRecord,
    RecoveryPublicationMessageRecord, RecoveryPublicationObligationRecord,
    RecoveryResourceViewRecord, RecoveryResultOutcomeRecord, RecoveryResultRecord,
    SendObligationDispositionRecord, SendObligationKindRecord, SessionLifecycleRecordAction,
    SessionProjectionRecord, ShutdownOutcomeRecord, ShutdownPlanRecord, ShutdownTargetKindRecord,
    ShutdownTargetRecord, ShutdownTargetRecoveryRecord, ShutdownTargetStateRecord,
    TerminalInterruptReasonRecord, TerminalResultRecord, WorkflowExecutionMetadataRecord,
    WorkflowExecutionProjectionRecord, WorkflowObligationRetirementReason,
    WorkflowObligationTerminalOutcome, WorkflowTurnCompletionObligationRecord,
    WorkflowTurnFailureSignalRecord, WorkflowWorktreeOwnerRecord,
};
pub use repository::{LocalEventSignal, LocalEventSubscription, LocalEventTransactionRepository};
