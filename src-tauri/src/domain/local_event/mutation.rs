//! Closed state-mutation family committed atomically with domain events.
//!
//! Every mutation is a compare-and-set: it names its key, the revision (or
//! content binding) it expects, and the complete new row. The store never
//! converts a guard mismatch into last-write-wins; a same-content replay
//! converges on the saved result, a different content is a typed conflict.

use crate::domain::local_event::events::ApplicationShutdownPhase;
use crate::domain::local_event::identifiers::{Revision, StreamId, StreamVersion};
use crate::domain::local_event::record::{
    MessageProjectionRecord, ObligationRecord, OperationReceiptRecord, OperationStatusRecord,
    RecoveryAttemptRecord, RecoveryResultRecord, SessionProjectionRecord, ShutdownPlanRecord,
    ShutdownTargetRecord, TerminalResultRecord, WorkflowExecutionProjectionRecord,
};
use crate::domain::workspace_tree::WorkspaceTreeNode;

/// A complete message projection may legitimately carry an inline image.
/// The transaction-wide decoded-byte admission remains the authoritative
/// 16 MiB ceiling, so one record may use that budget but cannot bypass it.
pub const RECOVERY_RESULT_MAX_BYTES: usize = 64 * 1024;

/// Operation kinds that carry a caller operation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Send,
    PermissionResponse,
    Stop,
    SessionLifecycle,
    ApplicationQuit,
}

impl OperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::PermissionResponse => "permission_response",
            Self::Stop => "stop",
            Self::SessionLifecycle => "session_lifecycle",
            Self::ApplicationQuit => "application_quit",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "send" => Some(Self::Send),
            "permission_response" => Some(Self::PermissionResponse),
            "stop" => Some(Self::Stop),
            "session_lifecycle" => Some(Self::SessionLifecycle),
            "application_quit" => Some(Self::ApplicationQuit),
            _ => None,
        }
    }
}

/// Revision guard for CAS rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionGuard {
    /// The row must not exist yet.
    Absent,
    /// The row must exist at exactly this revision.
    Expected(Revision),
}

/// Caller-scoped identity tuple `(principal, generation, kind, caller key)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallerOperationKey {
    pub principal: String,
    pub installation_id: String,
    pub kind: OperationKind,
    pub caller_request_id: String,
}

/// Immutable caller binding. Guard: key absent, or same HMAC / operation ID
/// (replay); a different binding is a payload conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBindingMutation {
    pub key: CallerOperationKey,
    pub operation_id: String,
    pub binding_hmac: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerAttemptResolution {
    Pending,
    Accepted,
    RejectedBeforeCommit,
    Cleared,
}

impl CallerAttemptResolution {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::RejectedBeforeCommit => "rejected_before_commit",
            Self::Cleared => "cleared",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "accepted" => Some(Self::Accepted),
            "rejected_before_commit" => Some(Self::RejectedBeforeCommit),
            "cleared" => Some(Self::Cleared),
            _ => None,
        }
    }
}

/// Built-in Tauri caller journal entry (owner-private local outbox).
/// Guard: key absent or same command hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerAttemptMutation {
    pub key: CallerOperationKey,
    /// Optional bounded owner scope used by built-in supervision queries.
    /// The exact command remains sealed and is never returned by that query.
    pub scope_id: Option<String>,
    pub command_hash: [u8; 32],
    /// Exact command encrypted / sealed by the caller-side journal owner.
    pub sealed_command: Vec<u8>,
    pub resolution: CallerAttemptResolution,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Direct operation record: immutable receipt plus mutable latest status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecordMutation {
    pub kind: OperationKind,
    pub operation_id: String,
    pub receipt: OperationReceiptRecord,
    pub latest_status: OperationStatusRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Complete bounded session / queue / lifecycle read-model row.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionProjectionMutation {
    pub session_id: String,
    pub projection: SessionProjectionRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Complete message / parts read-model row.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionMutation {
    pub session_id: String,
    pub message_id: String,
    pub projection: MessageProjectionRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Remove a newly-created session read model when its owning setup operation
/// rolls back before admission. Message rows are removed by the same mutation
/// before the session row, so no derived transcript can survive the rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProjectionRemovalMutation {
    pub session_id: String,
    pub expected: RevisionGuard,
}

/// Removes every durable Provider AgentSession payload except the newly
/// appended tombstone. A released provider ownership aggregate is removed in
/// the same transaction so the provider session can be claimed again without
/// retaining its resume identifier in Releash state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAgentSessionRemovalMutation {
    pub agent_session_stream: StreamId,
    pub retained_tombstone_sequence: StreamVersion,
    pub ownership_projection_id: Option<String>,
    pub ownership_stream: Option<StreamId>,
    pub ownership_expected: Option<Revision>,
}

/// One indexed execution row, guarded by the owning WorkflowExecution
/// aggregate revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionProjectionMutation {
    pub projection: WorkflowExecutionProjectionRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Complete node-row delta for one WorkflowExecution revision. Runtime writes
/// replace only this execution's rows; deleting an execution removes them via
/// the execution-row foreign key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExecutionNodeProjectionMutation {
    pub execution_id: String,
    pub nodes: Vec<WorkspaceTreeNode>,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Terminal uniqueness on `(session_id, turn_id)`. Guard: absent or same
/// terminal identity (replay); a different identity is a conflict.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalRecordMutation {
    pub session_id: String,
    pub turn_id: String,
    pub terminal_identity: String,
    pub result: TerminalResultRecord,
    pub participant_digest: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResolutionKind {
    Succeeded,
    Superseded,
}

impl StopResolutionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "succeeded" => Some(Self::Succeeded),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// Stop resolution keyed by Stop operation ID. Guard: absent or same result.
#[derive(Debug, Clone, PartialEq)]
pub struct StopResolutionMutation {
    pub stop_operation_id: String,
    pub resolution: StopResolutionKind,
    pub detail: TerminalResultRecord,
}

/// Recovery partition presented for unowned pending work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPartition {
    Owner,
    ClosedSession,
    ArchivedSession,
    UnownedRuntime,
}

impl PendingPartition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::ClosedSession => "closed_session",
            Self::ArchivedSession => "archived_session",
            Self::UnownedRuntime => "unowned_runtime",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "owner" => Some(Self::Owner),
            "closed_session" => Some(Self::ClosedSession),
            "archived_session" => Some(Self::ArchivedSession),
            "unowned_runtime" => Some(Self::UnownedRuntime),
            _ => None,
        }
    }
}

/// Membership row in the ordered pending-recovery index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingIndexEntry {
    /// Ordered key for bounded first-page reads.
    pub ordered_key: String,
    pub owner: String,
    pub partition: PendingPartition,
    pub shutdown_plan: Option<ShutdownPlanKey>,
}

/// Obligation state row and its pending-index membership, kept in parity
/// inside one transaction. `pending: Some(_)` inserts / keeps the index row,
/// `None` deletes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ObligationMutation {
    pub obligation_id: String,
    pub record: ObligationRecord,
    pub pending: Option<PendingIndexEntry>,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Recovery action attempt / immutable completed result, bound to the
/// backend-issued action ID. Guard: absent or same binding / expected
/// revision. Once `completed` is saved it never changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryActionMutation {
    pub action_id: String,
    pub binding_hash: [u8; 32],
    pub attempt: RecoveryAttemptRecord,
    pub completed: Option<RecoveryResultRecord>,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Shutdown aggregate identity. It is the accepted application-quit
/// operation identity; no second plan, epoch, or generation identity exists.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShutdownPlanKey {
    pub shutdown_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownDetailsState {
    Available,
    Compacted,
}

impl ShutdownDetailsState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Compacted => "compacted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "available" => Some(Self::Available),
            "compacted" => Some(Self::Compacted),
            _ => None,
        }
    }
}

/// Shutdown plan root: phase, bounded summary, details state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownPlanMutation {
    pub key: ShutdownPlanKey,
    pub phase: ApplicationShutdownPhase,
    pub summary: ShutdownPlanRecord,
    pub details_state: ShutdownDetailsState,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Bounded per-target detail row of a shutdown plan.
#[derive(Debug, Clone, PartialEq)]
pub struct ShutdownTargetMutation {
    pub key: ShutdownPlanKey,
    pub ordinal: i64,
    pub detail: ShutdownTargetRecord,
    pub expected: RevisionGuard,
    pub revision: Revision,
}

/// Frozen recovery detail captured for a shutdown plan (insert-once).
#[derive(Debug, Clone, PartialEq)]
pub struct ShutdownRecoverySnapshotMutation {
    pub key: ShutdownPlanKey,
    pub partition: PendingPartition,
    pub ordinal: i64,
    pub detail: ShutdownTargetRecord,
}

/// CAS on the store-wide "current shutdown plan" pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownLatestPointerMutation {
    pub expected: Option<ShutdownPlanKey>,
    pub new: Option<ShutdownPlanKey>,
}

/// Atomically converts one terminal shutdown to summary-only retention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownDetailsCompactionMutation {
    pub key: ShutdownPlanKey,
    pub expected: Revision,
    pub revision: Revision,
}

/// Closed mutation family from the issues-1499 design "State mutations".
/// Arbitrary SQL is not expressible through this port.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalStateMutation {
    OperationBinding(OperationBindingMutation),
    CallerAttempt(CallerAttemptMutation),
    OperationRecord(OperationRecordMutation),
    SessionProjection(SessionProjectionMutation),
    MessageProjection(MessageProjectionMutation),
    SessionProjectionRemoval(SessionProjectionRemovalMutation),
    ProviderAgentSessionRemoval(ProviderAgentSessionRemovalMutation),
    WorkflowExecutionProjection(WorkflowExecutionProjectionMutation),
    WorkflowExecutionNodeProjection(WorkflowExecutionNodeProjectionMutation),
    TerminalRecord(TerminalRecordMutation),
    StopResolution(StopResolutionMutation),
    Obligation(ObligationMutation),
    RecoveryAction(RecoveryActionMutation),
    /// Session lifecycle operations share the direct operation-record shape
    /// with `kind == OperationKind::SessionLifecycle`.
    SessionLifecycleOperation(OperationRecordMutation),
    ShutdownPlan(ShutdownPlanMutation),
    ShutdownTarget(ShutdownTargetMutation),
    ShutdownRecoverySnapshot(ShutdownRecoverySnapshotMutation),
    ShutdownDetailsCompaction(ShutdownDetailsCompactionMutation),
    ShutdownLatestPointer(ShutdownLatestPointerMutation),
}

impl LocalStateMutation {
    /// Stable, explicitly-versioned semantic bytes for commit idempotency.
    ///
    /// Generic projection commits currently accept only these mutation
    /// families. Keeping the match closed makes a newly-added family fail
    /// admission until its identity encoding is deliberately specified; Rust
    /// `Debug` output is never a persistent identity contract.
    pub fn canonical_identity_v1(&self) -> Result<Vec<u8>, &'static str> {
        fn field(bytes: &mut Vec<u8>, value: &[u8]) {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value);
        }
        fn text(bytes: &mut Vec<u8>, value: &str) {
            field(bytes, value.as_bytes());
        }
        fn revision_guard(bytes: &mut Vec<u8>, guard: RevisionGuard) {
            match guard {
                RevisionGuard::Absent => bytes.push(0),
                RevisionGuard::Expected(revision) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&revision.value().to_be_bytes());
                }
            }
        }
        fn revision(bytes: &mut Vec<u8>, value: Revision) {
            bytes.extend_from_slice(&value.value().to_be_bytes());
        }
        fn plan_key(bytes: &mut Vec<u8>, key: &ShutdownPlanKey) {
            text(bytes, &key.shutdown_id);
        }
        let mut bytes = b"local_state_mutation_identity_v1".to_vec();
        match self {
            // Projection identity is the canonical Stored*V1 representation,
            // which belongs to the persistence gateway. Calling this
            // domain-only encoder for a projection would silently change
            // existing replay identities, so projection-capable commit paths
            // must use the gateway canonicalizer.
            Self::SessionProjection(_)
            | Self::MessageProjection(_)
            | Self::WorkflowExecutionProjection(_)
            | Self::WorkflowExecutionNodeProjection(_) => {
                return Err("projection identity-v1 encoding is gateway-owned")
            }
            Self::SessionProjectionRemoval(m) => {
                text(&mut bytes, "session_projection_removal");
                text(&mut bytes, &m.session_id);
                revision_guard(&mut bytes, m.expected);
            }
            Self::ProviderAgentSessionRemoval(m) => {
                text(&mut bytes, "provider_agent_session_removal");
                text(&mut bytes, m.agent_session_stream.as_str());
                bytes.extend_from_slice(&m.retained_tombstone_sequence.value().to_be_bytes());
                match (
                    &m.ownership_projection_id,
                    &m.ownership_stream,
                    m.ownership_expected,
                ) {
                    (Some(projection_id), Some(stream), Some(expected)) => {
                        bytes.push(1);
                        text(&mut bytes, projection_id);
                        text(&mut bytes, stream.as_str());
                        revision(&mut bytes, expected);
                    }
                    (None, None, None) => bytes.push(0),
                    _ => return Err("incomplete provider ownership removal"),
                }
            }
            Self::TerminalRecord(m) => {
                text(&mut bytes, "terminal_record");
                text(&mut bytes, &m.session_id);
                text(&mut bytes, &m.turn_id);
                text(&mut bytes, &m.terminal_identity);
                m.result.write_canonical_identity_v1(&mut bytes)?;
                field(&mut bytes, &m.participant_digest);
            }
            Self::StopResolution(m) => {
                text(&mut bytes, "stop_resolution");
                text(&mut bytes, &m.stop_operation_id);
                text(&mut bytes, m.resolution.label());
                m.detail.write_canonical_identity_v1(&mut bytes)?;
            }
            Self::Obligation(m) => {
                text(&mut bytes, "obligation");
                text(&mut bytes, &m.obligation_id);
                m.record.write_canonical_identity_v1(&mut bytes)?;
                match &m.pending {
                    Some(pending) => {
                        bytes.push(1);
                        text(&mut bytes, &pending.ordered_key);
                        text(&mut bytes, &pending.owner);
                        text(&mut bytes, pending.partition.label());
                        match &pending.shutdown_plan {
                            Some(key) => {
                                bytes.push(1);
                                plan_key(&mut bytes, key);
                            }
                            None => bytes.push(0),
                        }
                    }
                    None => bytes.push(0),
                }
                revision_guard(&mut bytes, m.expected);
                revision(&mut bytes, m.revision);
            }
            Self::OperationBinding(_)
            | Self::CallerAttempt(_)
            | Self::OperationRecord(_)
            | Self::RecoveryAction(_)
            | Self::SessionLifecycleOperation(_)
            | Self::ShutdownPlan(_)
            | Self::ShutdownTarget(_)
            | Self::ShutdownRecoverySnapshot(_)
            | Self::ShutdownDetailsCompaction(_)
            | Self::ShutdownLatestPointer(_) => {
                return Err("mutation has no local-state identity-v1 encoding")
            }
        }
        Ok(bytes)
    }

    /// Approximate decoded size used by queue admission accounting.
    pub fn approximate_bytes(&self) -> usize {
        fn typed<T: std::fmt::Debug>(value: &T) -> usize {
            format!("{value:?}").len()
        }
        match self {
            Self::OperationBinding(m) => m.key.caller_request_id.len() + 96,
            Self::CallerAttempt(m) => {
                m.sealed_command.len() + m.scope_id.as_ref().map_or(0, String::len) + 128
            }
            Self::OperationRecord(m) | Self::SessionLifecycleOperation(m) => {
                typed(&m.receipt) + typed(&m.latest_status) + 64
            }
            Self::SessionProjection(m) => m.projection.semantic_bytes().saturating_add(64),
            Self::MessageProjection(m) => m.projection.semantic_bytes().saturating_add(96),
            Self::SessionProjectionRemoval(m) => m.session_id.len() + 64,
            Self::ProviderAgentSessionRemoval(m) => {
                m.agent_session_stream.as_str().len()
                    + m.ownership_projection_id.as_ref().map_or(0, String::len)
                    + m.ownership_stream
                        .as_ref()
                        .map_or(0, |stream| stream.as_str().len())
                    + 96
            }
            Self::WorkflowExecutionProjection(m) => typed(&m.projection).saturating_add(64),
            Self::WorkflowExecutionNodeProjection(m) => {
                m.nodes
                    .iter()
                    .fold(m.execution_id.len().saturating_add(64), |total, node| {
                        total
                            .saturating_add(node.id.len())
                            .saturating_add(node.title.len())
                            .saturating_add(node.error_reason.as_ref().map_or(0, String::len))
                            .saturating_add(node.display_command.as_ref().map_or(0, String::len))
                            .saturating_add(
                                node.command_result
                                    .as_ref()
                                    .map_or(0, |result| result.stdout.len() + result.stderr.len()),
                            )
                            .saturating_add(256)
                    })
            }
            Self::TerminalRecord(m) => typed(&m.result) + 128,
            Self::StopResolution(m) => typed(&m.detail) + 64,
            Self::Obligation(m) => {
                typed(&m.record)
                    + m.pending
                        .as_ref()
                        .map(|p| p.ordered_key.len() + p.owner.len() + 64)
                        .unwrap_or(0)
                    + 64
            }
            Self::RecoveryAction(m) => {
                typed(&m.attempt) + m.completed.as_ref().map(typed).unwrap_or(0) + 96
            }
            Self::ShutdownPlan(m) => typed(&m.summary) + 96,
            Self::ShutdownTarget(m) => typed(&m.detail) + 96,
            Self::ShutdownRecoverySnapshot(m) => typed(&m.detail) + 96,
            Self::ShutdownDetailsCompaction(_) => 96,
            Self::ShutdownLatestPointer(_) => 64,
        }
    }

    /// Whether this mutation belongs to the critical writer lane
    /// (terminal / Stop / shutdown closure work).
    pub fn is_critical(&self) -> bool {
        matches!(
            self,
            Self::TerminalRecord(_)
                | Self::StopResolution(_)
                | Self::ShutdownPlan(_)
                | Self::ShutdownTarget(_)
                | Self::ShutdownRecoverySnapshot(_)
                | Self::ShutdownDetailsCompaction(_)
                | Self::ShutdownLatestPointer(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_labels_round_trip() {
        for kind in [
            OperationKind::Send,
            OperationKind::PermissionResponse,
            OperationKind::Stop,
            OperationKind::SessionLifecycle,
            OperationKind::ApplicationQuit,
        ] {
            assert_eq!(OperationKind::parse(kind.label()), Some(kind));
        }
        for partition in [
            PendingPartition::Owner,
            PendingPartition::ClosedSession,
            PendingPartition::ArchivedSession,
            PendingPartition::UnownedRuntime,
        ] {
            assert_eq!(PendingPartition::parse(partition.label()), Some(partition));
        }
        assert_eq!(OperationKind::parse("unknown"), None);
    }

    #[test]
    fn canonical_identity_v1_is_a_stable_explicit_contract() {
        let mutation =
            LocalStateMutation::SessionProjectionRemoval(SessionProjectionRemovalMutation {
                session_id: "s-1".to_string(),
                expected: RevisionGuard::Expected(Revision::new(7).unwrap()),
            });

        assert_eq!(
            mutation.canonical_identity_v1().unwrap(),
            mutation.clone().canonical_identity_v1().unwrap()
        );

        let unsupported = LocalStateMutation::OperationBinding(OperationBindingMutation {
            key: CallerOperationKey {
                principal: "principal".to_string(),
                installation_id: "generation".to_string(),
                kind: OperationKind::Send,
                caller_request_id: "request".to_string(),
            },
            operation_id: "operation".to_string(),
            binding_hmac: [0; 32],
        });
        assert_eq!(
            unsupported.canonical_identity_v1(),
            Err("mutation has no local-state identity-v1 encoding")
        );
    }

    #[test]
    fn canonical_identity_v1_covers_terminal_and_obligation_semantics() {
        let terminal = LocalStateMutation::TerminalRecord(TerminalRecordMutation {
            session_id: "s-1".to_string(),
            turn_id: "turn-1".to_string(),
            terminal_identity: "terminal-1".to_string(),
            result: TerminalResultRecord::StopSuperseded {
                terminal_identity: "winner-1".to_string(),
                terminal_result_sha256: [1; 32],
            },
            participant_digest: [2; 32],
        });
        assert_eq!(
            terminal.canonical_identity_v1().unwrap(),
            terminal.clone().canonical_identity_v1().unwrap()
        );
        let mut different_terminal = terminal.clone();
        let LocalStateMutation::TerminalRecord(record) = &mut different_terminal else {
            unreachable!();
        };
        record.participant_digest = [3; 32];
        assert_ne!(
            terminal.canonical_identity_v1().unwrap(),
            different_terminal.canonical_identity_v1().unwrap()
        );

        let send = |state| ObligationRecord::Send {
            obligation_id: "ob-1".to_string(),
            operation_id: "op-1".to_string(),
            session_id: "s-1".to_string(),
            kind: crate::domain::local_event::record::SendObligationKindRecord::TurnExecution,
            disposition:
                crate::domain::local_event::record::SendObligationDispositionRecord::StartedTurn,
            human_message_id: None,
            assistant_message_id: None,
            reserved_turn_id: None,
            turn_id: Some("turn-1".to_string()),
            dependency_obligation_ids: Vec::new(),
            canonical_payload: "payload-ref".to_string(),
            state,
        };
        let obligation = |state| {
            LocalStateMutation::Obligation(ObligationMutation {
                obligation_id: "ob-1".to_string(),
                record: send(state),
                pending: None,
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            })
        };
        assert_ne!(
            obligation(crate::domain::local_event::record::ObligationStateRecord::Pending)
                .canonical_identity_v1()
                .unwrap(),
            obligation(crate::domain::local_event::record::ObligationStateRecord::Completed)
                .canonical_identity_v1()
                .unwrap()
        );
    }
}
