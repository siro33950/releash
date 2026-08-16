//! Permanent local event store domain.
//!
//! Owns the closed vocabulary of the single mutation authority spanning
//! agent sessions and workflows: store identities, the atomic batch, the
//! closed state-mutation family, the closed query sum, and the
//! `LocalEventTransactionRepository` port. No serde, rusqlite, filesystem,
//! Tauri, or WebSocket dependency is allowed here.

pub mod batch;
pub mod commit_admission;
pub mod events;
pub mod failure;
pub mod identifiers;
pub mod mutation;
pub mod operation_identity;
pub mod operation_record;
pub mod query;
pub mod record;
pub mod recovery;
#[allow(clippy::module_inception)]
pub mod repository;
pub mod workflow_shutdown;

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
    AgentSessionRemovalMutation, CallerAttemptMutation, CallerAttemptResolution,
    CallerOperationKey, LocalStateMutation, ObligationMutation, OperationBindingMutation,
    OperationKind, OperationRecordMutation, PendingIndexEntry, PendingPartition,
    RecoveryActionMutation, RevisionGuard, SessionProjectionMutation,
    SessionProjectionRemovalMutation, ShutdownDetailsCompactionMutation, ShutdownDetailsState,
    ShutdownLatestPointerMutation, ShutdownPlanKey, ShutdownPlanMutation,
    ShutdownRecoverySnapshotMutation, ShutdownTargetMutation,
    WorkflowExecutionNodeProjectionMutation, WorkflowExecutionProjectionMutation,
};
pub use operation_identity::{constant_time_eq_32, validate_operation_identity};
pub use operation_record::validate_operation_record;
pub use query::{
    AgentSessionOriginKind, AgentSessionProjectionPageView, CallerAttemptView,
    CanonicalRuntimeOwnerView, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    ObligationView, OperationBindingSummaryView, OperationBindingView, OperationRecordView,
    PendingIndexEntryView, PendingObligationView, PendingRecoveryPageView,
    PendingRecoverySnapshotPageView, QueryCursor, RecoveryActionView, SessionProjectionView,
    ShutdownPlanPageView, ShutdownPlanView, ShutdownSnapshotEntryView, ShutdownTargetView,
};
pub use record::{
    AgentSessionLifecycleRecord, AgentSessionOriginRecord, AgentSessionProjectionRecord,
    AgentSessionProviderRecord, ObligationRecord, ObligationStateRecord, OperationReceiptRecord,
    OperationStatusRecord, OperationStatusValue, ProviderHookHealthProjectionRecord,
    ProviderSessionOwnershipProjectionRecord, RecoveryAttemptRecord, RecoveryResourceViewRecord,
    RecoveryResultOutcomeRecord, RecoveryResultRecord, SessionProjectionRecord,
    ShutdownOutcomeRecord, ShutdownPlanRecord, ShutdownTargetKindRecord, ShutdownTargetRecord,
    ShutdownTargetRecoveryRecord, ShutdownTargetStateRecord, WorkflowExecutionMetadataRecord,
    WorkflowExecutionProjectionRecord, WorkflowWorktreeOwnerRecord,
};
pub use recovery::{RecoveryActionKind, RecoveryResultClassification};
pub use repository::LocalEventTransactionRepository;
