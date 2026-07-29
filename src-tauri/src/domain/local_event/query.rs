//! Closed query sum and its one-to-one result sum.
//!
//! Queries are snapshot point / range lookups over direct indexes and
//! projection tables; they never repair, migrate, or rebuild implicitly, and
//! they never return generic rows, JSON, or maps.

use std::fmt;

use crate::domain::local_event::batch::CommitResolution;
use crate::domain::local_event::events::{
    ApplicationShutdownPhase, DomainEventPage, LoadStreamRequest,
};
use crate::domain::local_event::failure::SafeOperationFailure;
use crate::domain::local_event::identifiers::{CommitIdentity, Revision};
use crate::domain::local_event::mutation::{
    CallerAttemptResolution, CallerOperationKey, OperationKind, PendingPartition,
    ShutdownDetailsState, ShutdownPlanKey, StopResolutionKind,
};
use crate::domain::local_event::record::{
    MessageProjectionRecord, ObligationRecord, OperationReceiptRecord, OperationStatusRecord,
    RecoveryAttemptRecord, RecoveryResultRecord, SessionProjectionRecord, ShutdownPlanRecord,
    ShutdownTargetRecord, TerminalResultRecord,
};

/// Opaque MAC-protected pagination cursor issued by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryCursor(String);

impl QueryCursor {
    /// Wrap an opaque cursor token received back from a caller. Integrity is
    /// verified by the store, not here.
    pub fn from_opaque(token: String) -> Self {
        Self(token)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Closed query sum. F8 / F10 additions become new variants together with
/// their schema evolution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum LocalEventQuery {
    CommitByIdentity {
        commit_id: CommitIdentity,
    },
    StreamPage(LoadStreamRequest),
    OperationByIdentity {
        kind: OperationKind,
        operation_id: String,
    },
    OperationBindingByIdentity {
        key: CallerOperationKey,
    },
    OperationBindingSummaryByOperation {
        installation_id: String,
        kind: OperationKind,
        operation_id: String,
        expected_binding_hmac: Option<[u8; 32]>,
    },
    CallerAttemptByIdentity {
        key: CallerOperationKey,
    },
    PendingCallerAttemptsByOperation {
        installation_id: String,
        kind: OperationKind,
        operation_id: String,
        limit: usize,
    },
    PendingCallerAttemptsByKind {
        installation_id: String,
        kind: OperationKind,
        limit: usize,
    },
    CallerAttemptPage {
        principal: String,
        installation_id: String,
        scope_id: String,
        limit: usize,
        after_kind: Option<OperationKind>,
        after_caller_request_id: Option<String>,
    },
    TerminalByTurn {
        session_id: String,
        turn_id: String,
    },
    StopResolutionByOperation {
        stop_operation_id: String,
    },
    ObligationByIdentity {
        obligation_id: String,
    },
    SessionProjectionByIdentity {
        session_id: String,
    },
    SessionProjectionPage {
        limit: usize,
        after_session_id: Option<String>,
    },
    /// One bounded, lightweight owner inventory read by a single SQLite
    /// statement. Startup GC uses this instead of composing independently
    /// snapshotted projection pages.
    CanonicalRuntimeOwnerSnapshot {
        limit: usize,
    },
    MessageProjectionByIdentity {
        session_id: String,
        message_id: String,
    },
    MessageProjectionPage {
        session_id: String,
        before_position: Option<i64>,
        limit: usize,
    },
    PendingRecoveryPage {
        limit: usize,
        partition: Option<PendingPartition>,
        /// Optional direct owner-index restriction used by the session-scoped
        /// feedback view. Public recovery discovery leaves this unset.
        owner: Option<String>,
        /// Optional ordered-key namespace restriction. With `owner` it
        /// isolates one session-scoped namespace; without `owner` it is the
        /// indexed startup selector for one durable effect kind.
        ordered_key_prefix: Option<String>,
        /// Exact current-inventory association selected by a shutdown plan.
        /// This never falls back to the frozen shutdown snapshot or the
        /// unfiltered inventory.
        shutdown_plan: Option<ShutdownPlanKey>,
        cursor: Option<QueryCursor>,
    },
    PendingRecoverySnapshotPage {
        plan: ShutdownPlanKey,
        snapshot_id: String,
        partition: PendingPartition,
        limit: usize,
        cursor: Option<QueryCursor>,
    },
    RecoveryActionByIdentity {
        action_id: String,
    },
    CurrentShutdown,
    /// Whether this exact current plan revision satisfies every durable
    /// RetryQuit precondition in one reader snapshot.
    RetryQuitEligibility {
        plan: ShutdownPlanKey,
        revision: Revision,
    },
    AvailableShutdownHistory {
        limit: usize,
    },
    ShutdownTargetByIdentity {
        plan: ShutdownPlanKey,
        ordinal: i64,
    },
    ShutdownPlanPage {
        plan: ShutdownPlanKey,
        limit: usize,
        cursor: Option<QueryCursor>,
    },
}

/// Saved operation record view (point lookup, never rebuilt from session
/// projections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecordView {
    pub kind: OperationKind,
    pub operation_id: String,
    pub receipt: OperationReceiptRecord,
    pub latest_status: OperationStatusRecord,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBindingSummaryView {
    pub total_count: usize,
    pub matching_binding_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBindingView {
    pub key: CallerOperationKey,
    pub operation_id: String,
    pub binding_hmac: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerAttemptView {
    pub key: CallerOperationKey,
    pub scope_id: Option<String>,
    pub operation_id: Option<String>,
    pub command_hash: [u8; 32],
    /// Owner-private encrypted retry material. Public page/presenter paths
    /// deliberately receive an empty vector; identity lookup is the only
    /// path that materializes it.
    pub sealed_command: Vec<u8>,
    pub resolution: CallerAttemptResolution,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalRecordView {
    pub session_id: String,
    pub turn_id: String,
    pub terminal_identity: String,
    pub result: TerminalResultRecord,
    pub participant_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionProjectionView {
    pub session_id: String,
    pub projection: SessionProjectionRecord,
    pub revision: Revision,
}

/// Lightweight runtime-ownership facts extracted from one canonical SQLite
/// statement. Inactive sessions remain present so a live PID can be resolved
/// to its canonical worktree without returning the full projection body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalRuntimeOwnerView {
    AgentSession {
        projection_id: String,
        session_id: String,
        worktree_path: String,
        active: bool,
        shutdown_target: bool,
        workflow_node_session: bool,
    },
    ActiveWorkflow {
        worktree_path: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionView {
    pub session_id: String,
    pub message_id: String,
    pub projection: MessageProjectionRecord,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionPageEntryView {
    pub position: i64,
    pub message: MessageProjectionView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionPageView {
    pub entries: Vec<MessageProjectionPageEntryView>,
    pub next_before_position: Option<i64>,
    pub total_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StopResolutionView {
    pub stop_operation_id: String,
    pub resolution: StopResolutionKind,
    pub detail: TerminalResultRecord,
}

/// Bounded owner lifecycle extracted by the gateway from a validated agent
/// session projection in the same SQLite snapshot as a pending obligation.
/// The persistence JSON never crosses the repository port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProjectionOwnerState {
    Normal,
    Closed,
    Archived,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingObligationView {
    pub obligation_id: String,
    pub ordered_key: String,
    pub owner: String,
    pub partition: PendingPartition,
    pub shutdown_plan: Option<ShutdownPlanKey>,
    pub record: ObligationRecord,
    /// SHA-256 of the exact validated StoredObligationV1 bytes. This lets a
    /// usecase bind a frozen snapshot without re-encoding persistence JSON.
    pub record_sha256: [u8; 32],
    /// Closed lifecycle extracted from the session projection captured in the
    /// same SQLite read snapshot as the pending row. `None` is exact absence,
    /// never a request to re-read current state outside the page snapshot.
    pub owner_projection: Option<SessionProjectionOwnerState>,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObligationView {
    pub obligation_id: String,
    pub record: ObligationRecord,
    pub record_sha256: [u8; 32],
    pub pending: Option<PendingIndexEntryView>,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingIndexEntryView {
    pub ordered_key: String,
    pub owner: String,
    pub partition: PendingPartition,
    pub shutdown_plan: Option<ShutdownPlanKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingRecoveryPageView {
    pub entries: Vec<PendingObligationView>,
    /// Cursor immediately after each entry in `entries`. This is kept out of
    /// the public DTO and lets the shared presenter stop at the exact encoded
    /// byte boundary without skipping source rows.
    pub continuation_cursors: Vec<QueryCursor>,
    pub next_cursor: Option<QueryCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShutdownSnapshotEntryView {
    pub plan: ShutdownPlanKey,
    pub partition: PendingPartition,
    pub ordinal: i64,
    pub detail: ShutdownTargetRecord,
    pub detail_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingRecoverySnapshotPageView {
    pub entries: Vec<ShutdownSnapshotEntryView>,
    /// Cursor immediately after each entry in `entries`; see
    /// `PendingRecoveryPageView::continuation_cursors`.
    pub continuation_cursors: Vec<QueryCursor>,
    pub next_cursor: Option<QueryCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionView {
    pub action_id: String,
    pub binding_hash: [u8; 32],
    pub attempt: RecoveryAttemptRecord,
    pub completed: Option<RecoveryResultRecord>,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownPlanView {
    pub plan: ShutdownPlanKey,
    pub phase: ApplicationShutdownPhase,
    pub summary: ShutdownPlanRecord,
    pub summary_sha256: [u8; 32],
    pub details_state: ShutdownDetailsState,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShutdownTargetView {
    pub plan: ShutdownPlanKey,
    pub ordinal: i64,
    pub detail: ShutdownTargetRecord,
    pub detail_sha256: [u8; 32],
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShutdownPlanPageView {
    pub plan: ShutdownPlanView,
    /// Empty with no next cursor when the plan details are `Compacted`.
    pub targets: Vec<ShutdownTargetView>,
    pub next_cursor: Option<QueryCursor>,
}

/// One-to-one result sum for [`LocalEventQuery`].
#[derive(Debug, Clone, PartialEq)]
pub enum LocalEventQueryResult {
    CommitByIdentity(CommitResolution),
    StreamPage(DomainEventPage),
    OperationByIdentity(Option<OperationRecordView>),
    OperationBindingByIdentity(Option<OperationBindingView>),
    OperationBindingSummaryByOperation(OperationBindingSummaryView),
    CallerAttemptByIdentity(Option<CallerAttemptView>),
    PendingCallerAttemptsByOperation(Vec<CallerAttemptView>),
    PendingCallerAttemptsByKind(Vec<CallerAttemptView>),
    CallerAttemptPage(Vec<CallerAttemptView>),
    TerminalByTurn(Option<TerminalRecordView>),
    StopResolutionByOperation(Option<StopResolutionView>),
    ObligationByIdentity(Option<ObligationView>),
    SessionProjectionByIdentity(Option<SessionProjectionView>),
    SessionProjectionPage(Vec<SessionProjectionView>),
    CanonicalRuntimeOwnerSnapshot(Vec<CanonicalRuntimeOwnerView>),
    MessageProjectionByIdentity(Option<MessageProjectionView>),
    MessageProjectionPage(MessageProjectionPageView),
    PendingRecoveryPage(PendingRecoveryPageView),
    PendingRecoverySnapshotPage(PendingRecoverySnapshotPageView),
    RecoveryActionByIdentity(Option<RecoveryActionView>),
    CurrentShutdown(Option<ShutdownPlanView>),
    RetryQuitEligibility(bool),
    AvailableShutdownHistory(Vec<ShutdownPlanView>),
    ShutdownTargetByIdentity(Option<ShutdownTargetView>),
    ShutdownPlanPage(ShutdownPlanPageView),
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum LocalEventQueryError {
    InvalidRequest,
    NotFound,
    CursorMismatch,
    CursorExpired,
    SnapshotMismatch,
    DetailsCompacted,
    QueryBusy,
    DeadlineExceeded,
    ResponseTooLarge,
    /// The requested replay window is no longer retained.
    ReplayRequired {
        correlation_id: String,
    },
    /// A stored event required for meaning could not be decoded.
    IncompatibleStoredEvent {
        correlation_id: String,
    },
    StorageUnavailable {
        failure: SafeOperationFailure,
    },
    Corrupt {
        correlation_id: String,
    },
    Internal {
        correlation_id: String,
    },
}

impl fmt::Display for LocalEventQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => write!(f, "invalid request"),
            Self::NotFound => write!(f, "not found"),
            Self::CursorMismatch => write!(f, "cursor mismatch"),
            Self::CursorExpired => write!(f, "cursor expired"),
            Self::SnapshotMismatch => write!(f, "snapshot mismatch"),
            Self::DetailsCompacted => write!(f, "details compacted"),
            Self::QueryBusy => write!(f, "query busy"),
            Self::DeadlineExceeded => write!(f, "deadline exceeded"),
            Self::ResponseTooLarge => write!(f, "response too large"),
            Self::ReplayRequired { correlation_id } => {
                write!(f, "replay required (correlation_id={correlation_id})")
            }
            Self::IncompatibleStoredEvent { correlation_id } => {
                write!(
                    f,
                    "incompatible stored event (correlation_id={correlation_id})"
                )
            }
            Self::StorageUnavailable { failure } => write!(f, "storage unavailable: {failure}"),
            Self::Corrupt { correlation_id } => {
                write!(f, "store corrupt (correlation_id={correlation_id})")
            }
            Self::Internal { correlation_id } => {
                write!(f, "internal error (correlation_id={correlation_id})")
            }
        }
    }
}

impl std::error::Error for LocalEventQueryError {}
