//! Bounded reader pool and the closed query implementations.
//!
//! Up to four dedicated reader threads own read-only connections; rusqlite
//! is never called on a tokio task. Every public query is a point / range
//! lookup over a direct index or projection table — never a scan of
//! `events` and never a full-history fold.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Condvar, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use crate::adaptor::gateway::local_event_store::clock::StoreClock;
use crate::adaptor::gateway::local_event_store::cursor::{
    filter_hash, issue_cursor, verify_cursor,
};
use crate::adaptor::gateway::local_event_store::envelope::{
    label_to_shutdown_phase, DecodedStoredEvent, EventCodecRegistry,
};
use crate::adaptor::gateway::local_event_store::projection_record_codec::decode_session_projection_record_v1;
use crate::adaptor::gateway::local_event_store::state_record_codec::{
    StoredObligationV1, StoredOperationReceiptV1, StoredOperationStatusV1, StoredRecoveryActionV1,
    StoredRecoveryResultV1, StoredShutdownPlanV1, StoredShutdownTargetV1,
};
use crate::domain::local_event::{
    validate_operation_record, CallerAttemptResolution, CallerAttemptView,
    CanonicalRuntimeOwnerView, CommitIdentity, CommittedDomainEvent, DomainEventPage, EventId,
    LoadStreamRequest, LoadedDomainEvent, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, ObligationView, OperationBindingView, OperationKind,
    OperationRecordView, PendingIndexEntryView, PendingObligationView, PendingPartition,
    PendingRecoveryPageView, PendingRecoverySnapshotPageView, QueryCursor, RecoveryActionView,
    SafeOperationFailure, SessionOperationFailureKind, SessionProjectionRecord,
    SessionProjectionView, ShutdownDetailsState, ShutdownPlanKey, ShutdownPlanPageView,
    ShutdownPlanView, ShutdownSnapshotEntryView, ShutdownTargetView, StreamSequence, StreamVersion,
};
use crate::domain::local_event::{
    ObligationRecord, OperationReceiptRecord, OperationStatusRecord, RecoveryAttemptRecord,
    RecoveryResultRecord, ShutdownPlanRecord, ShutdownTargetRecord,
};

pub const READER_POOL_SIZE: usize = 4;
pub const READ_QUEUE_MAX_DEPTH: usize = 128;
pub const QUERY_DEADLINE_MS: i64 = 2_000;
pub const CURSOR_TTL_MS: i64 = 5 * 60 * 1_000;

pub const MAX_PENDING_RECOVERY_PAGE: usize = 200;
/// The public 4 MiB bound is enforced after semantic decoding and DTO
/// expansion. This separate cap bounds one internal SQLite fetch without
/// confusing opaque record bytes with encoded public response bytes.
///
/// The internal cap stays an eighth of the public bound so that DTO expansion
/// keeps a whole internal page inside the public response. That matters beyond
/// response size: a page that survives to the public layer intact never needs
/// the public truncation path, and only an untruncated page lets a snapshot be
/// released as soon as its internal cursor is exhausted. Expansion is not
/// bounded in theory — an owner repeated across the entry and its target, or a
/// control character escaped as `\u00XX`, still multiplies bytes — so the
/// public truncation remains as a safety valve.
pub const PENDING_RECOVERY_INTERNAL_PAGE_MAX_BYTES: usize = 512 * 1024;
pub const MAX_ACTIVE_RECOVERY_SNAPSHOTS: usize = 8;
pub const MAX_SHUTDOWN_PAGE: usize = 128;
pub const SHUTDOWN_PAGE_MAX_BYTES: usize = 1024 * 1024;
pub const MAX_STREAM_PAGE: usize = 200;
pub const STREAM_PAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT: usize = 8_192;
/// Query plans for these statements are snapshot-gated in tests: they must
/// never contain a `SCAN events` step.
pub(crate) const SQL_PENDING_FIRST_PAGE: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.ordered_key > ?1 AND substr(po.ordered_key, 1, 9) <> 'feedback:' AND substr(po.ordered_key, 1, 19) <> 'workflow_execution:' ORDER BY po.ordered_key LIMIT ?2";
pub(crate) const SQL_PENDING_FIRST_PAGE_PARTITION: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.partition = ?1 AND po.ordered_key > ?2 AND substr(po.ordered_key, 1, 9) <> 'feedback:' AND substr(po.ordered_key, 1, 19) <> 'workflow_execution:' ORDER BY po.ordered_key LIMIT ?3";
pub(crate) const SQL_PENDING_FIRST_PAGE_OWNER: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po INDEXED BY idx_pending_obligations_owner JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.owner = ?1 AND po.ordered_key > ?2 AND substr(po.ordered_key, 1, 9) <> 'feedback:' AND substr(po.ordered_key, 1, 19) <> 'workflow_execution:' ORDER BY po.ordered_key LIMIT ?3";
pub(crate) const SQL_PENDING_FIRST_PAGE_OWNER_PREFIX: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.owner = ?1 AND po.ordered_key > ?2 AND po.ordered_key >= ?3 AND po.ordered_key < ?4 ORDER BY po.ordered_key LIMIT ?5";
pub(crate) const SQL_PENDING_FIRST_PAGE_PREFIX: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.ordered_key > ?1 AND po.ordered_key >= ?2 AND po.ordered_key < ?3 ORDER BY po.ordered_key LIMIT ?4";
pub(crate) const SQL_PENDING_FIRST_PAGE_SHUTDOWN_PLAN: &str = "SELECT po.obligation_id, po.ordered_key, po.owner, po.partition, po.shutdown_id, o.record, o.revision, sp.projection FROM pending_obligations po INDEXED BY idx_pending_obligations_shutdown JOIN obligations o ON o.obligation_id = po.obligation_id LEFT JOIN session_projection sp ON sp.session_id = po.owner WHERE po.shutdown_id = ?1 AND po.ordered_key > ?2 ORDER BY po.ordered_key LIMIT ?3";
pub(crate) const SQL_OPERATION_LOOKUP: &str = "SELECT receipt, latest_status, revision FROM operation_records WHERE kind = ?1 AND operation_id = ?2";

type CallerAttemptRow = (
    Option<String>,
    Option<String>,
    Vec<u8>,
    Vec<u8>,
    String,
    i64,
);
type ObligationRow = (
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub struct QueryContext {
    pub registry: Arc<EventCodecRegistry>,
    pub cursor_key: Vec<u8>,
    pub process_instance_id: String,
    pub clock: Arc<dyn StoreClock>,
}

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn storage_unavailable(error: &rusqlite::Error) -> LocalEventQueryError {
    // Concurrent-commit contention surfaces as SQLITE_BUSY / SQLITE_LOCKED
    // after the 250 ms busy timeout; that is `QueryBusy`, not a storage
    // failure (B-069).
    if let rusqlite::Error::SqliteFailure(inner, _) = error {
        if matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ) {
            return LocalEventQueryError::QueryBusy;
        }
    }
    let correlation = correlation_id();
    log::warn!("local event store read failure [{correlation}]: {error}");
    LocalEventQueryError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            true,
            "local event store read failed",
            correlation,
        ),
    }
}

fn corrupt(context: &str) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event store corrupt read [{correlation}]: {context}");
    LocalEventQueryError::Corrupt {
        correlation_id: correlation,
    }
}

fn reader_pool_unavailable(message: &'static str, retryable: bool) -> LocalEventQueryError {
    let correlation = correlation_id();
    log::error!("local event reader pool failure [{correlation}]: {message}");
    LocalEventQueryError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            retryable,
            message,
            correlation,
        ),
    }
}

fn session_projection_record(
    raw: String,
    session_id: &str,
    context: &'static str,
) -> Result<SessionProjectionRecord, LocalEventQueryError> {
    decode_session_projection_record_v1(&raw, session_id).map_err(|_| corrupt(context))
}

fn state_record<T>(
    result: Result<T, impl std::fmt::Debug>,
    context: &'static str,
) -> Result<T, LocalEventQueryError> {
    result.map_err(|_| {
        let correlation = correlation_id();
        log::error!("incompatible local state record [{correlation}]: {context}");
        LocalEventQueryError::IncompatibleStoredEvent {
            correlation_id: correlation,
        }
    })
}

fn operation_receipt_record(
    raw: &str,
    context: &'static str,
) -> Result<OperationReceiptRecord, LocalEventQueryError> {
    state_record(StoredOperationReceiptV1::decode(raw), context).map(|value| value.into_value())
}

fn operation_status_record(
    raw: &str,
    context: &'static str,
) -> Result<OperationStatusRecord, LocalEventQueryError> {
    state_record(StoredOperationStatusV1::decode(raw), context).map(|value| value.into_value())
}

fn obligation_record(
    raw: &str,
    context: &'static str,
) -> Result<ObligationRecord, LocalEventQueryError> {
    state_record(StoredObligationV1::decode(raw), context).map(|value| value.into_value())
}

fn recovery_attempt_record(
    raw: &str,
    context: &'static str,
) -> Result<RecoveryAttemptRecord, LocalEventQueryError> {
    state_record(StoredRecoveryActionV1::decode(raw), context).map(|value| value.into_value())
}

fn recovery_result_record(
    raw: &str,
    context: &'static str,
) -> Result<RecoveryResultRecord, LocalEventQueryError> {
    state_record(StoredRecoveryResultV1::decode(raw), context).map(|value| value.into_value())
}

fn shutdown_plan_record(
    raw: &str,
    context: &'static str,
) -> Result<ShutdownPlanRecord, LocalEventQueryError> {
    state_record(StoredShutdownPlanV1::decode(raw), context).map(|value| value.into_value())
}

fn shutdown_target_record(
    raw: &str,
    context: &'static str,
) -> Result<ShutdownTargetRecord, LocalEventQueryError> {
    state_record(StoredShutdownTargetV1::decode(raw), context).map(|value| value.into_value())
}

fn raw_sha256(raw: &str) -> [u8; 32] {
    Sha256::digest(raw.as_bytes()).into()
}

fn blob32(raw: Vec<u8>, context: &'static str) -> Result<[u8; 32], LocalEventQueryError> {
    raw.as_slice().try_into().map_err(|_| corrupt(context))
}

/// Execute one closed query on a reader connection.
pub fn run_query(
    connection: &Connection,
    context: &QueryContext,
    query: &LocalEventQuery,
) -> Result<LocalEventQueryResult, LocalEventQueryError> {
    run_query_in_recovery_snapshot(connection, context, query, None)
}

/// Execute a query while binding recovery cursors to one held SQLite read
/// transaction. Non-recovery queries ignore `recovery_snapshot_id`.
pub(crate) fn run_query_in_recovery_snapshot(
    connection: &Connection,
    context: &QueryContext,
    query: &LocalEventQuery,
    recovery_snapshot_id: Option<&str>,
) -> Result<LocalEventQueryResult, LocalEventQueryError> {
    match query {
        LocalEventQuery::OperationByIdentity { kind, operation_id } => {
            Ok(LocalEventQueryResult::OperationByIdentity(
                operation_by_identity(connection, *kind, operation_id)?,
            ))
        }
        LocalEventQuery::OperationBindingByIdentity { key } => {
            Ok(LocalEventQueryResult::OperationBindingByIdentity(
                operation_binding_by_identity(connection, key)?,
            ))
        }
        LocalEventQuery::OperationBindingSummaryByOperation {
            installation_id,
            kind,
            operation_id,
            expected_binding_hmac,
        } => Ok(LocalEventQueryResult::OperationBindingSummaryByOperation(
            operation_binding_summary_by_operation(
                connection,
                installation_id,
                *kind,
                operation_id,
                expected_binding_hmac.as_ref(),
            )?,
        )),
        LocalEventQuery::CallerAttemptByIdentity { key } => {
            Ok(LocalEventQueryResult::CallerAttemptByIdentity(
                caller_attempt_by_identity(connection, key)?,
            ))
        }
        LocalEventQuery::PendingCallerAttemptsByOperation {
            installation_id,
            kind,
            operation_id,
            limit,
        } => Ok(LocalEventQueryResult::PendingCallerAttemptsByOperation(
            pending_caller_attempts(
                connection,
                installation_id,
                *kind,
                Some(operation_id),
                *limit,
            )?,
        )),
        LocalEventQuery::PendingCallerAttemptsByKind {
            installation_id,
            kind,
            limit,
        } => Ok(LocalEventQueryResult::PendingCallerAttemptsByKind(
            pending_caller_attempts(connection, installation_id, *kind, None, *limit)?,
        )),
        LocalEventQuery::CallerAttemptPage {
            principal,
            installation_id,
            scope_id,
            limit,
            after_kind,
            after_caller_request_id,
        } => Ok(LocalEventQueryResult::CallerAttemptPage(
            caller_attempt_page(
                connection,
                principal,
                installation_id,
                scope_id,
                *limit,
                *after_kind,
                after_caller_request_id.as_deref(),
            )?,
        )),
        LocalEventQuery::ObligationByIdentity { obligation_id } => {
            Ok(LocalEventQueryResult::ObligationByIdentity(
                obligation_by_identity(connection, obligation_id)?,
            ))
        }
        LocalEventQuery::SessionProjectionByIdentity { session_id } => {
            Ok(LocalEventQueryResult::SessionProjectionByIdentity(
                session_projection_by_identity(connection, session_id)?,
            ))
        }
        LocalEventQuery::CanonicalRuntimeOwnerSnapshot { limit } => {
            Ok(LocalEventQueryResult::CanonicalRuntimeOwnerSnapshot(
                canonical_runtime_owner_snapshot(connection, *limit)?,
            ))
        }
        LocalEventQuery::PendingRecoveryPage {
            limit,
            partition,
            owner,
            ordered_key_prefix,
            shutdown_plan,
            cursor,
        } => Ok(LocalEventQueryResult::PendingRecoveryPage(
            pending_recovery_page(
                connection,
                context,
                PendingRecoveryPageRequest {
                    limit: *limit,
                    partition: *partition,
                    owner: owner.as_deref(),
                    ordered_key_prefix: ordered_key_prefix.as_deref(),
                    shutdown_plan: shutdown_plan.as_ref(),
                    cursor: cursor.as_ref(),
                    query_snapshot_id: recovery_snapshot_id,
                },
            )?,
        )),
        LocalEventQuery::PendingRecoverySnapshotPage {
            plan,
            snapshot_id,
            partition,
            limit,
            cursor,
        } => Ok(LocalEventQueryResult::PendingRecoverySnapshotPage(
            pending_recovery_snapshot_page(
                connection,
                context,
                PendingRecoverySnapshotPageRequest {
                    plan,
                    snapshot_id,
                    partition: *partition,
                    limit: *limit,
                    cursor: cursor.as_ref(),
                    query_snapshot_id: recovery_snapshot_id,
                },
            )?,
        )),
        LocalEventQuery::RecoveryActionByIdentity { action_id } => {
            Ok(LocalEventQueryResult::RecoveryActionByIdentity(
                recovery_action_by_identity(connection, action_id)?,
            ))
        }
        LocalEventQuery::CurrentShutdown => Ok(LocalEventQueryResult::CurrentShutdown(
            current_shutdown(connection, context)?,
        )),
        LocalEventQuery::RetryQuitEligibility { plan, revision } => {
            Ok(LocalEventQueryResult::RetryQuitEligibility(
                retry_quit_eligibility(connection, context, plan, *revision)?,
            ))
        }
        LocalEventQuery::AvailableShutdownHistory { limit } => {
            Ok(LocalEventQueryResult::AvailableShutdownHistory(
                available_shutdown_history(connection, *limit)?,
            ))
        }
        LocalEventQuery::ShutdownTargetByIdentity { plan, ordinal } => {
            Ok(LocalEventQueryResult::ShutdownTargetByIdentity(
                shutdown_target_by_identity(connection, plan, *ordinal)?,
            ))
        }
        LocalEventQuery::ShutdownPlanPage {
            plan,
            limit,
            cursor,
        } => Ok(LocalEventQueryResult::ShutdownPlanPage(shutdown_plan_page(
            connection,
            context,
            plan,
            *limit,
            cursor.as_ref(),
        )?)),
    }
}

fn operation_binding_by_identity(
    connection: &Connection,
    key: &crate::domain::local_event::CallerOperationKey,
) -> Result<Option<OperationBindingView>, LocalEventQueryError> {
    let row: Option<(String, Vec<u8>)> = connection
        .query_row(
            "SELECT operation_id, binding_hmac FROM operation_bindings
             WHERE principal = ?1 AND installation_id = ?2 AND kind = ?3
               AND caller_request_id = ?4",
            params![
                key.principal,
                key.installation_id,
                key.kind.label(),
                key.caller_request_id
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(|(operation_id, binding_hmac)| {
        Ok(OperationBindingView {
            key: key.clone(),
            operation_id,
            binding_hmac: blob32(binding_hmac, "operation binding hmac")?,
        })
    })
    .transpose()
}

fn caller_attempt_by_identity(
    connection: &Connection,
    key: &crate::domain::local_event::CallerOperationKey,
) -> Result<Option<CallerAttemptView>, LocalEventQueryError> {
    let row: Option<CallerAttemptRow> = connection
        .query_row(
            "SELECT a.scope_id, b.operation_id, a.command_hash, a.sealed_command,
                    a.resolution, a.revision
             FROM caller_attempts a
             LEFT JOIN operation_bindings b
               ON b.principal = a.principal AND b.installation_id = a.installation_id
              AND b.kind = a.kind AND b.caller_request_id = a.caller_request_id
             WHERE a.principal = ?1 AND a.installation_id = ?2 AND a.kind = ?3
               AND a.caller_request_id = ?4",
            params![
                key.principal,
                key.installation_id,
                key.kind.label(),
                key.caller_request_id
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(
        |(scope_id, operation_id, hash, sealed_command, resolution, revision)| {
            Ok(CallerAttemptView {
                key: key.clone(),
                scope_id,
                operation_id,
                command_hash: blob32(hash, "caller attempt command hash")?,
                sealed_command,
                resolution: CallerAttemptResolution::parse(&resolution)
                    .ok_or_else(|| corrupt("caller attempt resolution"))?,
                revision: crate::domain::local_event::Revision::new(revision)
                    .map_err(|_| corrupt("caller attempt revision"))?,
            })
        },
    )
    .transpose()
}

fn pending_caller_attempts(
    connection: &Connection,
    installation_id: &str,
    kind: OperationKind,
    operation_id: Option<&str>,
    limit: usize,
) -> Result<Vec<CallerAttemptView>, LocalEventQueryError> {
    if installation_id.is_empty()
        || operation_id.is_some_and(str::is_empty)
        || limit == 0
        || limit > 16
    {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let mut statement = connection
        .prepare(
            "SELECT a.principal, a.caller_request_id, a.scope_id, b.operation_id,
                    a.command_hash, a.sealed_command, a.resolution, a.revision
             FROM caller_attempts AS a INDEXED BY idx_caller_attempts_pending_kind
             INNER JOIN operation_bindings AS b INDEXED BY idx_operation_bindings_operation
               ON b.principal = a.principal AND b.installation_id = a.installation_id
              AND b.kind = a.kind AND b.caller_request_id = a.caller_request_id
             WHERE a.installation_id = ?1 AND a.kind = ?2 AND a.resolution = 'pending'
               AND (?3 IS NULL OR b.operation_id = ?3)
             ORDER BY a.principal, a.caller_request_id
             LIMIT ?4",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(
            params![installation_id, kind.label(), operation_id, limit as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .map_err(|error| storage_unavailable(&error))?;
    rows.map(|row| {
        let (
            principal,
            caller_request_id,
            scope_id,
            operation_id,
            command_hash,
            sealed_command,
            resolution,
            revision,
        ) = row.map_err(|error| storage_unavailable(&error))?;
        Ok(CallerAttemptView {
            key: crate::domain::local_event::CallerOperationKey {
                principal,
                installation_id: installation_id.to_string(),
                kind,
                caller_request_id,
            },
            scope_id,
            operation_id: Some(operation_id),
            command_hash: blob32(command_hash, "pending caller attempt command hash")?,
            sealed_command,
            resolution: CallerAttemptResolution::parse(&resolution)
                .ok_or_else(|| corrupt("pending caller attempt resolution"))?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("pending caller attempt revision"))?,
        })
    })
    .collect()
}

fn caller_attempt_page(
    connection: &Connection,
    principal: &str,
    installation_id: &str,
    scope_id: &str,
    limit: usize,
    after_kind: Option<OperationKind>,
    after_caller_request_id: Option<&str>,
) -> Result<Vec<CallerAttemptView>, LocalEventQueryError> {
    if principal.is_empty()
        || installation_id.is_empty()
        || scope_id.is_empty()
        || limit == 0
        || limit > 128
        || after_kind.is_some() != after_caller_request_id.is_some()
    {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let after_kind = after_kind.map(OperationKind::label).unwrap_or("");
    let after_id = after_caller_request_id.unwrap_or("");
    let mut statement = connection
        .prepare(
            "SELECT a.kind, a.caller_request_id, b.operation_id,
                    a.command_hash, a.resolution, a.revision
             FROM caller_attempts AS a INDEXED BY idx_caller_attempts_scope
             LEFT JOIN operation_bindings b
               ON b.principal = a.principal AND b.installation_id = a.installation_id
              AND b.kind = a.kind AND b.caller_request_id = a.caller_request_id
             WHERE a.principal = ?1 AND a.installation_id = ?2 AND a.scope_id = ?3
               AND a.resolution <> 'cleared'
               AND (a.kind > ?4 OR (a.kind = ?4 AND a.caller_request_id > ?5))
             ORDER BY a.kind, a.caller_request_id LIMIT ?6",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(
            params![
                principal,
                installation_id,
                scope_id,
                after_kind,
                after_id,
                limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(|error| storage_unavailable(&error))?;
    rows.map(|row| {
        let (kind, caller_request_id, operation_id, command_hash, resolution, revision) =
            row.map_err(|error| storage_unavailable(&error))?;
        Ok(CallerAttemptView {
            key: crate::domain::local_event::CallerOperationKey {
                principal: principal.to_string(),
                installation_id: installation_id.to_string(),
                kind: OperationKind::parse(&kind).ok_or_else(|| corrupt("caller attempt kind"))?,
                caller_request_id,
            },
            scope_id: Some(scope_id.to_string()),
            operation_id,
            command_hash: blob32(command_hash, "caller attempt command hash")?,
            sealed_command: Vec::new(),
            resolution: CallerAttemptResolution::parse(&resolution)
                .ok_or_else(|| corrupt("caller attempt resolution"))?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("caller attempt revision"))?,
        })
    })
    .collect()
}

pub fn load_stream_page(
    connection: &Connection,
    context: &QueryContext,
    request: &LoadStreamRequest,
) -> Result<DomainEventPage, LocalEventQueryError> {
    if request.limit == 0 {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let limit = request.limit.min(MAX_STREAM_PAGE);
    let after = request.after.map(|sequence| sequence.value()).unwrap_or(0);
    let head: Option<i64> = connection
        .query_row(
            "SELECT head FROM stream_heads WHERE stream_id = ?1",
            params![request.stream_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let head = StreamVersion::new(head.unwrap_or(0)).map_err(|_| corrupt("stream head"))?;

    let mut statement = connection
        .prepare(
            "SELECT event_id, commit_id, stream_sequence, global_sequence, event_type,
                    payload_version, occurred_at, payload
             FROM events
             WHERE stream_id = ?1 AND stream_sequence > ?2
             ORDER BY stream_sequence
             LIMIT ?3",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(
            params![request.stream_id.as_str(), after, limit as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                ))
            },
        )
        .map_err(|error| storage_unavailable(&error))?;

    let mut events = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let (
            event_id,
            commit_id,
            stream_sequence,
            global_sequence,
            event_type,
            payload_version,
            occurred_at,
            payload,
        ) = row.map_err(|error| storage_unavailable(&error))?;
        if !events.is_empty() && bytes + payload.len() > STREAM_PAGE_MAX_BYTES {
            break;
        }
        bytes += payload.len();
        let decoded = context
            .registry
            .decode(&event_type, payload_version, &payload)
            .map_err(|_| LocalEventQueryError::IncompatibleStoredEvent {
                correlation_id: correlation_id(),
            })?;
        let event = match decoded {
            DecodedStoredEvent::Known(event) => LoadedDomainEvent::Known(event),
            DecodedStoredEvent::Unknown => LoadedDomainEvent::Unknown {
                event_type: event_type.clone(),
                payload_version,
            },
        };
        events.push(CommittedDomainEvent {
            event_id: EventId::parse(&event_id).map_err(|_| corrupt("stored event id"))?,
            commit_id: CommitIdentity::parse(&commit_id)
                .map_err(|_| corrupt("stored commit id"))?,
            stream_id: request.stream_id.clone(),
            stream_sequence: StreamSequence::new(stream_sequence)
                .map_err(|_| corrupt("stored stream sequence"))?,
            global_sequence: crate::domain::local_event::GlobalSequence::new(global_sequence)
                .map_err(|_| corrupt("stored global sequence"))?,
            occurred_at_ms: occurred_at
                .parse()
                .map_err(|_| corrupt("stored occurred_at"))?,
            event,
        });
    }
    let next_after = events
        .last()
        .filter(|event| event.stream_sequence.value() < head.value())
        .map(|event| event.stream_sequence);
    Ok(DomainEventPage {
        events,
        head,
        next_after,
    })
}

fn operation_by_identity(
    connection: &Connection,
    kind: OperationKind,
    operation_id: &str,
) -> Result<Option<OperationRecordView>, LocalEventQueryError> {
    let row: Option<(String, String, i64)> = connection
        .query_row(
            SQL_OPERATION_LOOKUP,
            params![kind.label(), operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(|(receipt, latest_status, revision)| {
        let receipt = operation_receipt_record(&receipt, "operation receipt")?;
        let latest_status = operation_status_record(&latest_status, "operation latest status")?;
        validate_operation_record(kind, operation_id, &receipt, &latest_status)
            .map_err(|_| corrupt("operation record aggregate"))?;
        Ok(OperationRecordView {
            kind,
            operation_id: operation_id.to_string(),
            receipt,
            latest_status,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("operation revision"))?,
        })
    })
    .transpose()
}

fn operation_binding_summary_by_operation(
    connection: &Connection,
    installation_id: &str,
    kind: OperationKind,
    operation_id: &str,
    expected_binding_hmac: Option<&[u8; 32]>,
) -> Result<crate::domain::local_event::OperationBindingSummaryView, LocalEventQueryError> {
    if installation_id.is_empty() || operation_id.is_empty() {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let expected_binding_hmac = expected_binding_hmac.map(|binding| binding.to_vec());
    let (total_count, matching_binding_count): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE
                        WHEN ?4 IS NOT NULL AND binding_hmac = ?4 THEN 1 ELSE 0
                    END), 0)
             FROM operation_bindings INDEXED BY idx_operation_bindings_operation
             WHERE installation_id = ?1 AND kind = ?2 AND operation_id = ?3",
            params![
                installation_id,
                kind.label(),
                operation_id,
                expected_binding_hmac
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    Ok(crate::domain::local_event::OperationBindingSummaryView {
        total_count: usize::try_from(total_count)
            .map_err(|_| corrupt("operation binding count"))?,
        matching_binding_count: usize::try_from(matching_binding_count)
            .map_err(|_| corrupt("matching operation binding count"))?,
    })
}

fn obligation_by_identity(
    connection: &Connection,
    obligation_id: &str,
) -> Result<Option<ObligationView>, LocalEventQueryError> {
    let row: Option<ObligationRow> = connection
        .query_row(
            "SELECT o.record, o.revision, po.ordered_key, po.owner, po.partition,
                    po.shutdown_id
             FROM obligations o
             LEFT JOIN pending_obligations po ON po.obligation_id = o.obligation_id
             WHERE o.obligation_id = ?1",
            params![obligation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(
        |(record, revision, ordered_key, owner, partition, shutdown_id)| {
            let pending = match (ordered_key, owner, partition) {
                (Some(ordered_key), Some(owner), Some(partition)) => Some(PendingIndexEntryView {
                    ordered_key,
                    owner,
                    partition: PendingPartition::parse(&partition)
                        .ok_or_else(|| corrupt("obligation partition tag"))?,
                    shutdown_plan: shutdown_id.map(|shutdown_id| ShutdownPlanKey { shutdown_id }),
                }),
                (None, None, None) => {
                    if shutdown_id.is_some() {
                        return Err(corrupt("detached obligation shutdown association"));
                    }
                    None
                }
                _ => return Err(corrupt("partial obligation pending index")),
            };
            let record_sha256 = raw_sha256(&record);
            Ok(ObligationView {
                obligation_id: obligation_id.to_string(),
                record: obligation_record(&record, "obligation record")?,
                record_sha256,
                pending,
                revision: crate::domain::local_event::Revision::new(revision)
                    .map_err(|_| corrupt("obligation revision"))?,
            })
        },
    )
    .transpose()
}

fn session_projection_by_identity(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<SessionProjectionView>, LocalEventQueryError> {
    let row: Option<(String, i64)> = connection
        .query_row(
            "SELECT projection, revision FROM session_projection WHERE session_id = ?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(|(projection, revision)| {
        Ok(SessionProjectionView {
            session_id: session_id.to_string(),
            projection: session_projection_record(projection, session_id, "session projection")?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("session projection revision"))?,
        })
    })
    .transpose()
}

fn canonical_runtime_owner_snapshot(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<CanonicalRuntimeOwnerView>, LocalEventQueryError> {
    use crate::adaptor::gateway::workflow::fact_log::record_from_row;
    use crate::domain::workflow::services::fact_replay;
    use crate::domain::workflow::{NodeFact, TreeRootFact};

    if limit == 0 || limit > MAX_CANONICAL_RUNTIME_OWNER_SNAPSHOT {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    // 統一 Node 事実ログの木ごとの fold で生存 owner を導出する。
    let roots = super::node_events::list_tree_roots(connection, "started")
        .map_err(|error| storage_unavailable(&error))?;
    if roots.len() > limit {
        return Err(LocalEventQueryError::ResponseTooLarge);
    }
    let mut owners = Vec::new();
    for root in roots {
        let records = super::node_events::read_tree(connection, &root.tree_id)
            .map_err(|error| storage_unavailable(&error))?
            .iter()
            .map(record_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| LocalEventQueryError::InvalidRequest)?;
        let Some(NodeFact::Started(started)) = records.first().map(|record| &record.fact) else {
            continue;
        };
        match &started.root {
            Some(TreeRootFact::Session(session_root)) => {
                let view = fact_replay::derive_session_facts(
                    &records,
                    &root.node_execution_id,
                    &root.tree_id,
                );
                owners.push(CanonicalRuntimeOwnerView::AgentSession {
                    worktree_path: session_root.worktree_path.clone(),
                    active: view.is_open(),
                });
            }
            Some(TreeRootFact::Workflow(workflow_root)) => {
                let Ok(Some(folded)) = fact_replay::fold_execution_tree(&root.tree_id, &records)
                else {
                    continue;
                };
                let read_model = fact_replay::derive_read_model(&folded);
                if read_model.status.is_active() {
                    owners.push(CanonicalRuntimeOwnerView::ActiveWorkflow {
                        worktree_path: workflow_root.worktree_path.clone(),
                    });
                }
            }
            None => {}
        }
    }
    Ok(owners)
}

type PendingRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<String>,
);

struct PendingRecoveryPageRequest<'a> {
    limit: usize,
    partition: Option<PendingPartition>,
    owner: Option<&'a str>,
    ordered_key_prefix: Option<&'a str>,
    shutdown_plan: Option<&'a ShutdownPlanKey>,
    cursor: Option<&'a QueryCursor>,
    query_snapshot_id: Option<&'a str>,
}

fn pending_recovery_filter(
    partition: Option<PendingPartition>,
    owner: Option<&str>,
    ordered_key_prefix: Option<&str>,
    shutdown_plan: Option<&ShutdownPlanKey>,
) -> [u8; 32] {
    filter_hash(&[
        "pending_recovery",
        partition
            .map(|partition| partition.label())
            .unwrap_or("all"),
        owner.unwrap_or("all_owners"),
        ordered_key_prefix.unwrap_or("all_namespaces"),
        shutdown_plan
            .map(|plan| plan.shutdown_id.as_str())
            .unwrap_or("all_shutdown_plans"),
    ])
}

fn pending_recovery_snapshot_filter(
    plan: &ShutdownPlanKey,
    snapshot_id: &str,
    partition: PendingPartition,
) -> [u8; 32] {
    filter_hash(&[
        "recovery_snapshot",
        &plan.shutdown_id,
        snapshot_id,
        partition.label(),
    ])
}

/// Verify an incoming recovery cursor before routing it to the connection
/// that owns its SQLite snapshot. A valid but no-longer-retained snapshot is
/// classified by the pager as `CursorExpired`.
pub(crate) fn recovery_query_snapshot_id(
    context: &QueryContext,
    query: &LocalEventQuery,
) -> Result<Option<String>, LocalEventQueryError> {
    let (cursor, filter) = match query {
        LocalEventQuery::PendingRecoveryPage {
            partition,
            owner,
            ordered_key_prefix,
            shutdown_plan,
            cursor: Some(cursor),
            ..
        } => (
            cursor,
            pending_recovery_filter(
                *partition,
                owner.as_deref(),
                ordered_key_prefix.as_deref(),
                shutdown_plan.as_ref(),
            ),
        ),
        LocalEventQuery::PendingRecoverySnapshotPage {
            plan,
            snapshot_id,
            partition,
            cursor: Some(cursor),
            ..
        } => (
            cursor,
            pending_recovery_snapshot_filter(plan, snapshot_id, *partition),
        ),
        LocalEventQuery::PendingRecoveryPage { cursor: None, .. }
        | LocalEventQuery::PendingRecoverySnapshotPage { cursor: None, .. } => return Ok(None),
        _ => return Ok(None),
    };
    verify_cursor(
        &context.cursor_key,
        cursor.as_str(),
        &filter,
        &context.process_instance_id,
        context.clock.now_ms(),
    )
    .map(|claims| Some(claims.snapshot_id))
}

fn ordered_key_prefix_end(prefix: &str) -> Result<String, LocalEventQueryError> {
    let mut chars = prefix.chars().collect::<Vec<_>>();
    while let Some(last) = chars.pop() {
        let mut next = u32::from(last).saturating_add(1);
        if next == 0xd800 {
            next = 0xe000;
        }
        if let Some(next) = char::from_u32(next) {
            chars.push(next);
            return Ok(chars.into_iter().collect());
        }
    }
    Err(LocalEventQueryError::InvalidRequest)
}

fn pending_recovery_page(
    connection: &Connection,
    context: &QueryContext,
    request: PendingRecoveryPageRequest<'_>,
) -> Result<PendingRecoveryPageView, LocalEventQueryError> {
    let PendingRecoveryPageRequest {
        limit,
        partition,
        owner,
        ordered_key_prefix,
        shutdown_plan,
        cursor,
        query_snapshot_id,
    } = request;
    if limit == 0 || limit > MAX_PENDING_RECOVERY_PAGE {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    if usize::from(partition.is_some())
        + usize::from(owner.is_some())
        + usize::from(shutdown_plan.is_some())
        > 1
        || owner.is_some_and(str::is_empty)
        || ordered_key_prefix.is_some_and(str::is_empty)
    {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let filter = pending_recovery_filter(partition, owner, ordered_key_prefix, shutdown_plan);
    let now_ms = context.clock.now_ms();
    let (last_key, cursor_snapshot_id, cursor_expiry_ms) = match cursor {
        Some(cursor) => {
            let claims = verify_cursor(
                &context.cursor_key,
                cursor.as_str(),
                &filter,
                &context.process_instance_id,
                now_ms,
            )?;
            if query_snapshot_id.is_some_and(|expected| expected != claims.snapshot_id) {
                return Err(LocalEventQueryError::CursorMismatch);
            }
            (claims.last_key, claims.snapshot_id, claims.expires_at_ms)
        }
        None => (
            String::new(),
            query_snapshot_id.unwrap_or("pending_recovery").to_string(),
            now_ms.saturating_add(CURSOR_TTL_MS),
        ),
    };

    let fetch = (limit + 1) as i64;
    let mut rows: Vec<PendingRow> = Vec::new();
    let mut internal_bytes = 0usize;
    let mut internal_truncated = false;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<PendingRow> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ))
    };
    macro_rules! collect_pending_rows {
        ($mapped:expr) => {
            for row in $mapped {
                let row = row.map_err(|error| storage_unavailable(&error))?;
                let row_bytes = row
                    .0
                    .len()
                    .saturating_add(row.1.len())
                    .saturating_add(row.2.len())
                    .saturating_add(row.3.len())
                    .saturating_add(row.4.as_ref().map_or(0, String::len))
                    .saturating_add(row.5.len())
                    .saturating_add(row.7.as_ref().map_or(0, String::len))
                    .saturating_add(std::mem::size_of::<i64>() * 2);
                if !rows.is_empty()
                    && internal_bytes.saturating_add(row_bytes)
                        > PENDING_RECOVERY_INTERNAL_PAGE_MAX_BYTES
                {
                    internal_truncated = true;
                    break;
                }
                internal_bytes = internal_bytes.saturating_add(row_bytes);
                rows.push(row);
            }
        };
    }
    match (partition, owner, ordered_key_prefix, shutdown_plan) {
        (None, None, None, Some(plan)) => {
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE_SHUTDOWN_PLAN)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![plan.shutdown_id, last_key, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        (None, Some(owner), Some(prefix), None) => {
            let prefix_end = ordered_key_prefix_end(prefix)?;
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE_OWNER_PREFIX)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![owner, last_key, prefix, prefix_end, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        (None, None, Some(prefix), None) => {
            let prefix_end = ordered_key_prefix_end(prefix)?;
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE_PREFIX)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![last_key, prefix, prefix_end, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        (None, Some(owner), None, None) => {
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE_OWNER)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![owner, last_key, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        (Some(partition), None, None, None) => {
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE_PARTITION)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![partition.label(), last_key, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        (None, None, None, None) => {
            let mut statement = connection
                .prepare(SQL_PENDING_FIRST_PAGE)
                .map_err(|error| storage_unavailable(&error))?;
            let mapped = statement
                .query_map(params![last_key, fetch], map_row)
                .map_err(|error| storage_unavailable(&error))?;
            collect_pending_rows!(mapped);
        }
        _ => return Err(LocalEventQueryError::InvalidRequest),
    }

    let has_more_rows = rows.len() > limit || internal_truncated;
    rows.truncate(limit);
    let mut entries = Vec::with_capacity(rows.len());
    let mut continuation_cursors = Vec::with_capacity(rows.len());
    for (
        obligation_id,
        ordered_key,
        owner,
        partition_label,
        shutdown_id,
        record,
        revision,
        owner_projection,
    ) in rows
    {
        continuation_cursors.push(QueryCursor::from_opaque(issue_cursor(
            &context.cursor_key,
            &cursor_snapshot_id,
            &filter,
            &ordered_key,
            &context.process_instance_id,
            cursor_expiry_ms,
        )));
        let record_sha256 = raw_sha256(&record);
        let _ = owner_projection;
        entries.push(PendingObligationView {
            obligation_id,
            ordered_key,
            owner,
            partition: PendingPartition::parse(&partition_label)
                .ok_or_else(|| corrupt("pending partition tag"))?,
            shutdown_plan: shutdown_id.map(|shutdown_id| ShutdownPlanKey { shutdown_id }),
            record: obligation_record(&record, "pending obligation record")?,
            record_sha256,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("pending obligation revision"))?,
        });
    }
    let next_cursor = if has_more_rows {
        continuation_cursors.last().cloned()
    } else {
        None
    };
    Ok(PendingRecoveryPageView {
        entries,
        continuation_cursors,
        next_cursor,
    })
}

fn load_plan(
    connection: &Connection,
    plan: &ShutdownPlanKey,
) -> Result<Option<ShutdownPlanView>, LocalEventQueryError> {
    let row: Option<(String, String, String, i64)> = connection
        .query_row(
            "SELECT phase, summary, details_state, revision
             FROM shutdown_plans
             WHERE shutdown_id = ?1",
            params![plan.shutdown_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    if let Some((phase, plan_summary, details_state, revision)) = row {
        let details_state = ShutdownDetailsState::parse(&details_state)
            .ok_or_else(|| corrupt("shutdown details state tag"))?;
        let summary_sha256 = raw_sha256(&plan_summary);
        return Ok(Some(ShutdownPlanView {
            plan: plan.clone(),
            phase: label_to_shutdown_phase(&phase).ok_or_else(|| corrupt("shutdown phase tag"))?,
            summary: shutdown_plan_record(&plan_summary, "shutdown plan summary")?,
            summary_sha256,
            details_state,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("shutdown plan revision"))?,
        }));
    }
    Ok(None)
}

fn available_shutdown_history(
    connection: &Connection,
    limit: usize,
) -> Result<Vec<ShutdownPlanView>, LocalEventQueryError> {
    if limit == 0 || limit > 3 {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let mut statement = connection
        .prepare(
            "SELECT shutdown_id, phase, summary, details_state, revision
             FROM shutdown_plans INDEXED BY idx_shutdown_plans_details_state
             WHERE details_state = 'available'
             ORDER BY rowid
             LIMIT ?1",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let rows = statement
        .query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|error| storage_unavailable(&error))?;
    rows.map(|row| {
        let (shutdown_id, phase, summary, details_state, revision) =
            row.map_err(|error| storage_unavailable(&error))?;
        let summary_sha256 = raw_sha256(&summary);
        Ok(ShutdownPlanView {
            plan: ShutdownPlanKey { shutdown_id },
            phase: label_to_shutdown_phase(&phase).ok_or_else(|| corrupt("shutdown phase tag"))?,
            summary: shutdown_plan_record(&summary, "shutdown plan summary")?,
            summary_sha256,
            details_state: ShutdownDetailsState::parse(&details_state)
                .ok_or_else(|| corrupt("shutdown details state tag"))?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("shutdown plan revision"))?,
        })
    })
    .collect()
}

struct PendingRecoverySnapshotPageRequest<'a> {
    plan: &'a ShutdownPlanKey,
    snapshot_id: &'a str,
    partition: PendingPartition,
    limit: usize,
    cursor: Option<&'a QueryCursor>,
    query_snapshot_id: Option<&'a str>,
}

fn pending_recovery_snapshot_page(
    connection: &Connection,
    context: &QueryContext,
    request: PendingRecoverySnapshotPageRequest<'_>,
) -> Result<PendingRecoverySnapshotPageView, LocalEventQueryError> {
    let PendingRecoverySnapshotPageRequest {
        plan,
        snapshot_id,
        partition,
        limit,
        cursor,
        query_snapshot_id,
    } = request;
    if limit == 0
        || limit > MAX_PENDING_RECOVERY_PAGE
        || snapshot_id.is_empty()
        || partition == PendingPartition::Owner
    {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let plan_view = load_plan(connection, plan)?.ok_or(LocalEventQueryError::NotFound)?;
    if plan_view.details_state == ShutdownDetailsState::Compacted {
        return Err(LocalEventQueryError::DetailsCompacted);
    }
    let stored_snapshot_id = plan_view
        .summary
        .recovery_snapshot_id
        .as_deref()
        .ok_or(LocalEventQueryError::SnapshotMismatch)?;
    if stored_snapshot_id != snapshot_id {
        return Err(LocalEventQueryError::SnapshotMismatch);
    }
    let filter = pending_recovery_snapshot_filter(plan, snapshot_id, partition);
    let now_ms = context.clock.now_ms();
    let (last_key, cursor_snapshot_id, cursor_expiry_ms) = match cursor {
        Some(cursor) => {
            let claims = verify_cursor(
                &context.cursor_key,
                cursor.as_str(),
                &filter,
                &context.process_instance_id,
                now_ms,
            )?;
            if query_snapshot_id.is_some_and(|expected| expected != claims.snapshot_id) {
                return Err(LocalEventQueryError::CursorMismatch);
            }
            (claims.last_key, claims.snapshot_id, claims.expires_at_ms)
        }
        None => (
            String::new(),
            query_snapshot_id.unwrap_or("recovery_snapshot").to_string(),
            now_ms.saturating_add(CURSOR_TTL_MS),
        ),
    };

    let mut statement = connection
        .prepare(
            "SELECT ordinal, detail FROM shutdown_recovery_snapshots
             WHERE shutdown_id = ?1 AND partition = ?2
               AND printf('%019d', ordinal) > ?3
             ORDER BY ordinal
             LIMIT ?4",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let mapped = statement
        .query_map(
            params![
                plan.shutdown_id,
                partition.label(),
                last_key,
                (limit + 1) as i64
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let mut rows = Vec::new();
    let mut internal_bytes = 0usize;
    let mut internal_truncated = false;
    for row in mapped {
        let row = row.map_err(|error| storage_unavailable(&error))?;
        let row_bytes = row.1.len().saturating_add(std::mem::size_of::<i64>());
        if !rows.is_empty()
            && internal_bytes.saturating_add(row_bytes) > PENDING_RECOVERY_INTERNAL_PAGE_MAX_BYTES
        {
            internal_truncated = true;
            break;
        }
        internal_bytes = internal_bytes.saturating_add(row_bytes);
        rows.push(row);
    }
    let has_more_rows = rows.len() > limit || internal_truncated;
    rows.truncate(limit);

    let mut entries = Vec::with_capacity(rows.len());
    let mut continuation_cursors = Vec::with_capacity(rows.len());
    for (ordinal, detail) in rows {
        let detail_sha256 = raw_sha256(&detail);
        let key = format!("{ordinal:019}");
        continuation_cursors.push(QueryCursor::from_opaque(issue_cursor(
            &context.cursor_key,
            &cursor_snapshot_id,
            &filter,
            &key,
            &context.process_instance_id,
            cursor_expiry_ms,
        )));
        entries.push(ShutdownSnapshotEntryView {
            plan: plan.clone(),
            partition,
            ordinal,
            detail: shutdown_target_record(&detail, "shutdown snapshot detail")?,
            detail_sha256,
        });
    }
    let next_cursor = if has_more_rows {
        continuation_cursors.last().cloned()
    } else {
        None
    };
    Ok(PendingRecoverySnapshotPageView {
        entries,
        continuation_cursors,
        next_cursor,
    })
}

fn recovery_action_by_identity(
    connection: &Connection,
    action_id: &str,
) -> Result<Option<RecoveryActionView>, LocalEventQueryError> {
    let row: Option<(Vec<u8>, String, Option<String>, i64)> = connection
        .query_row(
            "SELECT binding_hash, attempt, completed, revision
             FROM recovery_action_attempts WHERE action_id = ?1",
            params![action_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    row.map(|(binding_hash, attempt, completed, revision)| {
        Ok(RecoveryActionView {
            action_id: action_id.to_string(),
            binding_hash: blob32(binding_hash, "recovery binding hash")?,
            attempt: recovery_attempt_record(&attempt, "recovery attempt")?,
            completed: completed
                .map(|payload| recovery_result_record(&payload, "recovery completed result"))
                .transpose()?,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("recovery action revision"))?,
        })
    })
    .transpose()
}

fn current_shutdown(
    connection: &Connection,
    context: &QueryContext,
) -> Result<Option<ShutdownPlanView>, LocalEventQueryError> {
    let pointer: Option<String> = connection
        .query_row(
            "SELECT current_shutdown_id
             FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let Some(shutdown_id) = pointer else {
        return Ok(None);
    };
    let key = ShutdownPlanKey { shutdown_id };
    let mut plan = load_plan(connection, &key)?
        .ok_or_else(|| corrupt("current shutdown pointer without plan row"))?;
    if matches!(
        plan.phase,
        crate::domain::local_event::ApplicationShutdownPhase::Prepared
            | crate::domain::local_event::ApplicationShutdownPhase::Activated
            | crate::domain::local_event::ApplicationShutdownPhase::Quiescing
    ) && plan.summary.process_instance_id != context.process_instance_id
    {
        plan.phase = crate::domain::local_event::ApplicationShutdownPhase::ReconciliationRequired;
    }
    Ok(Some(plan))
}

fn retry_quit_eligibility(
    connection: &Connection,
    context: &QueryContext,
    plan: &ShutdownPlanKey,
    revision: crate::domain::local_event::Revision,
) -> Result<bool, LocalEventQueryError> {
    type RetryEvidence = (
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Option<RetryEvidence> = connection
        .query_row(
            "SELECT m.health, m.process_instance_id, p.phase, p.summary, p.commit_id,
                    o.receipt, o.latest_status, o.commit_id
             FROM store_metadata AS m
             JOIN shutdown_plans AS p
               ON p.shutdown_id = m.current_shutdown_id
             LEFT JOIN operation_records AS o
               ON o.kind = 'application_quit'
              AND o.operation_id = CASE
                    WHEN json_valid(p.summary) THEN json_extract(p.summary, '$.operation_id')
                    ELSE NULL
                  END
             WHERE m.id = 1 AND p.shutdown_id = ?1 AND p.revision = ?2",
            params![plan.shutdown_id, revision.value()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let Some((
        health,
        metadata_boot_id,
        phase,
        summary,
        plan_commit_id,
        receipt,
        latest_status,
        operation_commit_id,
    )) = row
    else {
        return Ok(false);
    };
    let summary: serde_json::Value =
        serde_json::from_str(&summary).map_err(|_| corrupt("retry quit shutdown summary"))?;
    let operation_id = summary
        .get("operation_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| corrupt("retry quit operation reference"))?;
    let (Some(receipt), Some(latest_status), Some(operation_commit_id)) =
        (receipt, latest_status, operation_commit_id)
    else {
        return Err(corrupt("retry quit operation record missing"));
    };
    let receipt: serde_json::Value =
        serde_json::from_str(&receipt).map_err(|_| corrupt("retry quit operation receipt"))?;
    let latest_status: serde_json::Value =
        serde_json::from_str(&latest_status).map_err(|_| corrupt("retry quit operation status"))?;
    let receipt_matches = receipt.get("schema").and_then(serde_json::Value::as_str)
        == Some("application_quit_receipt_v1")
        && receipt
            .get("operation_id")
            .and_then(serde_json::Value::as_str)
            == Some(operation_id)
        && receipt
            .get("shutdown_id")
            .and_then(serde_json::Value::as_str)
            == Some(plan.shutdown_id.as_str());
    if !receipt_matches {
        return Err(corrupt("retry quit operation receipt reference"));
    }
    if latest_status
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some("application_quit_status_v1")
    {
        return Err(corrupt("retry quit operation status schema"));
    }
    let status_type = latest_status
        .get("state")
        .and_then(|state| state.get("type"))
        .and_then(serde_json::Value::as_str);
    if status_type.is_none() {
        return Err(corrupt("retry quit operation status state"));
    }
    Ok(health == "ok"
        && metadata_boot_id == context.process_instance_id
        && phase == "failed"
        && summary
            .get("process_instance_id")
            .and_then(serde_json::Value::as_str)
            == Some(context.process_instance_id.as_str())
        && summary.get("outcome").and_then(serde_json::Value::as_str)
            == Some("aborted_before_activation")
        && summary
            .get("shutdown_effect_count")
            .and_then(serde_json::Value::as_u64)
            == Some(0)
        && summary
            .get("admission_open")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && summary
            .get("retry_quit_same_boot")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && status_type == Some("failed_before_activation")
        && plan_commit_id == operation_commit_id)
}

fn shutdown_plan_page(
    connection: &Connection,
    context: &QueryContext,
    plan: &ShutdownPlanKey,
    limit: usize,
    cursor: Option<&QueryCursor>,
) -> Result<ShutdownPlanPageView, LocalEventQueryError> {
    if limit == 0 || limit > MAX_SHUTDOWN_PAGE {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let plan_view = load_plan(connection, plan)?.ok_or(LocalEventQueryError::NotFound)?;
    if plan_view.details_state == ShutdownDetailsState::Compacted {
        // The compacted plan keeps its identity / phase / summary; entries
        // are empty with no next cursor.
        return Ok(ShutdownPlanPageView {
            plan: plan_view,
            targets: Vec::new(),
            next_cursor: None,
        });
    }
    let filter = filter_hash(&["shutdown_targets", &plan.shutdown_id]);
    let now_ms = context.clock.now_ms();
    let last_ordinal: i64 = match cursor {
        Some(cursor) => verify_cursor(
            &context.cursor_key,
            cursor.as_str(),
            &filter,
            &context.process_instance_id,
            now_ms,
        )?
        .last_key
        .parse()
        .map_err(|_| LocalEventQueryError::CursorMismatch)?,
        None => -1,
    };

    let mut statement = connection
        .prepare(
            "SELECT ordinal, detail, revision FROM shutdown_targets
             WHERE shutdown_id = ?1 AND ordinal > ?2
             ORDER BY ordinal
             LIMIT ?3",
        )
        .map_err(|error| storage_unavailable(&error))?;
    let mapped = statement
        .query_map(
            params![plan.shutdown_id, last_ordinal, (limit + 1) as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(|error| storage_unavailable(&error))?;
    let mut rows = Vec::new();
    for row in mapped {
        rows.push(row.map_err(|error| storage_unavailable(&error))?);
    }
    let has_more_rows = rows.len() > limit;
    rows.truncate(limit);

    let mut targets = Vec::new();
    let mut bytes = 0usize;
    let mut truncated_by_bytes = false;
    for (ordinal, detail, revision) in rows {
        if targets.is_empty() && detail.len() > SHUTDOWN_PAGE_MAX_BYTES {
            return Err(LocalEventQueryError::ResponseTooLarge);
        }
        if !targets.is_empty() && bytes + detail.len() > SHUTDOWN_PAGE_MAX_BYTES {
            truncated_by_bytes = true;
            break;
        }
        bytes += detail.len();
        let detail_sha256 = raw_sha256(&detail);
        targets.push(ShutdownTargetView {
            plan: plan.clone(),
            ordinal,
            detail: shutdown_target_record(&detail, "shutdown target detail")?,
            detail_sha256,
            revision: crate::domain::local_event::Revision::new(revision)
                .map_err(|_| corrupt("shutdown target revision"))?,
        });
    }
    let next_cursor = if has_more_rows || truncated_by_bytes {
        targets.last().map(|target| {
            QueryCursor::from_opaque(issue_cursor(
                &context.cursor_key,
                "shutdown_targets",
                &filter,
                &target.ordinal.to_string(),
                &context.process_instance_id,
                now_ms + CURSOR_TTL_MS,
            ))
        })
    } else {
        None
    };
    Ok(ShutdownPlanPageView {
        plan: plan_view,
        targets,
        next_cursor,
    })
}

fn shutdown_target_by_identity(
    connection: &Connection,
    plan: &ShutdownPlanKey,
    ordinal: i64,
) -> Result<Option<ShutdownTargetView>, LocalEventQueryError> {
    if ordinal < 0 {
        return Err(LocalEventQueryError::InvalidRequest);
    }
    let plan_view = load_plan(connection, plan)?.ok_or(LocalEventQueryError::NotFound)?;
    if plan_view.details_state == ShutdownDetailsState::Compacted {
        return Err(LocalEventQueryError::DetailsCompacted);
    }
    connection
        .query_row(
            "SELECT detail, revision FROM shutdown_targets
             WHERE shutdown_id = ?1 AND ordinal = ?2",
            params![plan.shutdown_id, ordinal],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?
        .map(|(detail, revision)| {
            if detail.len() > SHUTDOWN_PAGE_MAX_BYTES {
                return Err(LocalEventQueryError::ResponseTooLarge);
            }
            let detail_sha256 = raw_sha256(&detail);
            Ok(ShutdownTargetView {
                plan: plan.clone(),
                ordinal,
                detail: shutdown_target_record(&detail, "shutdown target detail")?,
                detail_sha256,
                revision: crate::domain::local_event::Revision::new(revision)
                    .map_err(|_| corrupt("shutdown target revision"))?,
            })
        })
        .transpose()
}

// --- Reader pool ---

/// The second argument is `true` when the job's deadline already passed and
/// the query must answer `DeadlineExceeded` without touching the database.
type ReadTask = Box<dyn FnOnce(&Connection, bool) + Send>;

struct ReadJob {
    deadline_ms: i64,
    task: ReadTask,
}

struct ReadQueueState {
    jobs: VecDeque<ReadJob>,
    closed: bool,
}

/// Shared bounded job queue feeding the dedicated reader threads.
pub struct ReaderPool {
    state: Mutex<ReadQueueState>,
    available: Condvar,
    clock: Arc<dyn StoreClock>,
    #[cfg(test)]
    running_workers: std::sync::atomic::AtomicUsize,
}

impl ReaderPool {
    pub fn new(clock: Arc<dyn StoreClock>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ReadQueueState {
                jobs: VecDeque::new(),
                closed: false,
            }),
            available: Condvar::new(),
            clock,
            #[cfg(test)]
            running_workers: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Submit a query; `QueryBusy` when the bounded queue is full.
    pub fn submit<T, F>(
        &self,
        run: F,
    ) -> Result<oneshot::Receiver<Result<T, LocalEventQueryError>>, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let (reply, receiver) = oneshot::channel();
        let deadline_ms = self.clock.now_ms() + QUERY_DEADLINE_MS;
        let mut state = self.state.lock().expect("reader queue poisoned");
        if state.closed {
            return Err(reader_pool_unavailable(
                "local event store reader pool is closed",
                false,
            ));
        }
        if state.jobs.len() >= READ_QUEUE_MAX_DEPTH {
            return Err(LocalEventQueryError::QueryBusy);
        }
        state.jobs.push_back(ReadJob {
            deadline_ms,
            task: Box::new(move |connection, deadline_exceeded| {
                if deadline_exceeded {
                    let _ = reply.send(Err(LocalEventQueryError::DeadlineExceeded));
                    return;
                }
                let _ = reply.send(run(connection));
            }),
        });
        drop(state);
        self.available.notify_one();
        Ok(receiver)
    }

    /// Synchronous facade over the same fixed reader workers.
    ///
    /// This exists for established synchronous application ports. It does not
    /// create a thread, runtime, or SQLite connection per call.
    pub fn submit_blocking<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let (reply, receiver) = mpsc::sync_channel(1);
        let deadline_ms = self.clock.now_ms() + QUERY_DEADLINE_MS;
        let mut state = self.state.lock().expect("reader queue poisoned");
        if state.closed {
            return Err(reader_pool_unavailable(
                "local event store reader pool is closed",
                false,
            ));
        }
        if state.jobs.len() >= READ_QUEUE_MAX_DEPTH {
            return Err(LocalEventQueryError::QueryBusy);
        }
        state.jobs.push_back(ReadJob {
            deadline_ms,
            task: Box::new(move |connection, deadline_exceeded| {
                let result = if deadline_exceeded {
                    Err(LocalEventQueryError::DeadlineExceeded)
                } else {
                    run(connection)
                };
                let _ = reply.send(result);
            }),
        });
        drop(state);
        self.available.notify_one();
        receiver
            .recv()
            .map_err(|_| reader_pool_unavailable("local event store reader reply lost", true))?
    }

    fn pop_blocking(&self) -> Option<ReadJob> {
        let mut state = self.state.lock().expect("reader queue poisoned");
        loop {
            if let Some(job) = state.jobs.pop_front() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state = self.available.wait(state).expect("reader queue poisoned");
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().expect("reader queue poisoned");
        state.closed = true;
        state.jobs.clear();
        drop(state);
        self.available.notify_all();
    }

    /// Worker loop for one dedicated reader thread.
    pub fn run_worker(self: &Arc<Self>, connection: Connection) {
        #[cfg(test)]
        self.running_workers
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        while let Some(job) = self.pop_blocking() {
            let deadline_exceeded = self.clock.now_ms() > job.deadline_ms;
            (job.task)(&connection, deadline_exceeded);
        }
        #[cfg(test)]
        self.running_workers
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

// --- Snapshot-stable recovery pager ---

struct RecoverySnapshotJob {
    query: LocalEventQuery,
    reply: oneshot::Sender<Result<LocalEventQueryResult, LocalEventQueryError>>,
}

struct RecoverySnapshotHandle {
    sender: mpsc::Sender<RecoverySnapshotJob>,
    expires_at_ms: i64,
}

struct RecoverySnapshotState {
    active: HashMap<String, RecoverySnapshotHandle>,
    issue_order: VecDeque<String>,
    closed: bool,
}

/// A bounded set of held SQLite read transactions for recovery cursors.
///
/// Each active cursor family owns one read-only connection and therefore one
/// SQLite snapshot. The pager retains at most `MAX_ACTIVE_RECOVERY_SNAPSHOTS`
/// transactions and never copies or fully materializes the pending inventory.
/// Eviction is an explicit retention expiry: a still-authentic cursor then
/// resolves to `CursorExpired` rather than falling back to current rows.
pub struct RecoverySnapshotPager {
    database_path: PathBuf,
    context: Arc<QueryContext>,
    state: Mutex<RecoverySnapshotState>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl RecoverySnapshotPager {
    pub fn new(database_path: PathBuf, context: Arc<QueryContext>) -> Arc<Self> {
        Arc::new(Self {
            database_path,
            context,
            state: Mutex::new(RecoverySnapshotState {
                active: HashMap::new(),
                issue_order: VecDeque::new(),
                closed: false,
            }),
            workers: Mutex::new(Vec::new()),
        })
    }

    fn reap_finished_workers(&self) {
        let finished = {
            let mut workers = self
                .workers
                .lock()
                .expect("recovery snapshot worker list poisoned");
            let mut finished = Vec::new();
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    finished.push(workers.swap_remove(index));
                } else {
                    index += 1;
                }
            }
            finished
        };
        for worker in finished {
            let _ = worker.join();
        }
    }

    fn unavailable(context: &'static str, error: impl std::fmt::Display) -> LocalEventQueryError {
        let correlation = correlation_id();
        log::warn!("recovery snapshot pager failure [{correlation}] ({context}): {error}");
        LocalEventQueryError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                true,
                "recovery snapshot read failed",
                correlation,
            ),
        }
    }

    fn remove(&self, snapshot_id: &str) {
        let mut state = self.state.lock().expect("recovery snapshot pager poisoned");
        Self::forget(&mut state, snapshot_id);
    }

    fn forget(state: &mut RecoverySnapshotState, snapshot_id: &str) {
        state.active.remove(snapshot_id);
        state.issue_order.retain(|id| id != snapshot_id);
    }

    fn reserve(
        &self,
        snapshot_id: &str,
        expires_at_ms: i64,
        sender: mpsc::Sender<RecoverySnapshotJob>,
    ) -> Result<(), LocalEventQueryError> {
        let now_ms = self.context.clock.now_ms();
        let mut state = self.state.lock().expect("recovery snapshot pager poisoned");
        if state.closed {
            return Err(Self::unavailable("closed", "pager is closed"));
        }
        let expired = state
            .active
            .iter()
            .filter(|(_, handle)| now_ms > handle.expires_at_ms)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            Self::forget(&mut state, &id);
        }
        while state.active.len() >= MAX_ACTIVE_RECOVERY_SNAPSHOTS {
            let Some(oldest) = state.issue_order.pop_front() else {
                break;
            };
            if state.active.remove(&oldest).is_some() {
                break;
            }
        }
        state.issue_order.push_back(snapshot_id.to_string());
        state.active.insert(
            snapshot_id.to_string(),
            RecoverySnapshotHandle {
                sender,
                expires_at_ms,
            },
        );
        Ok(())
    }

    fn should_retain(result: &Result<LocalEventQueryResult, LocalEventQueryError>) -> bool {
        match result {
            Ok(LocalEventQueryResult::PendingRecoveryPage(page)) => page.next_cursor.is_some(),
            Ok(LocalEventQueryResult::PendingRecoverySnapshotPage(page)) => {
                page.next_cursor.is_some()
            }
            _ => false,
        }
    }

    async fn dispatch(
        &self,
        snapshot_id: &str,
        sender: &mpsc::Sender<RecoverySnapshotJob>,
        query: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        let (reply, receiver) = oneshot::channel();
        if sender.send(RecoverySnapshotJob { query, reply }).is_err() {
            self.remove(snapshot_id);
            return Err(LocalEventQueryError::CursorExpired);
        }
        let result = receiver.await.unwrap_or_else(|_| {
            Err(Self::unavailable(
                "worker_reply",
                "snapshot worker stopped before replying",
            ))
        });
        if !Self::should_retain(&result) {
            self.remove(snapshot_id);
        }
        result
    }

    pub async fn query(
        self: &Arc<Self>,
        query: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        if let Some(snapshot_id) = recovery_query_snapshot_id(&self.context, &query)? {
            let sender = {
                let state = self.state.lock().expect("recovery snapshot pager poisoned");
                state
                    .active
                    .get(&snapshot_id)
                    .map(|handle| handle.sender.clone())
            }
            .ok_or(LocalEventQueryError::CursorExpired)?;
            return self.dispatch(&snapshot_id, &sender, query).await;
        }

        let snapshot_id = uuid::Uuid::new_v4().to_string();
        let expires_at_ms = self.context.clock.now_ms().saturating_add(CURSOR_TTL_MS);
        let (sender, receiver) = mpsc::channel::<RecoverySnapshotJob>();
        self.reap_finished_workers();
        self.reserve(&snapshot_id, expires_at_ms, sender.clone())?;

        let database_path = self.database_path.clone();
        let context = Arc::clone(&self.context);
        let worker_snapshot_id = snapshot_id.clone();
        let worker = match std::thread::Builder::new()
            .name(format!(
                "local-event-recovery-snapshot-{}",
                &snapshot_id[..8]
            ))
            .spawn(move || {
                let connection =
                    match crate::adaptor::gateway::local_event_store::connection::open_reader(
                        &database_path,
                    ) {
                        Ok(connection) => connection,
                        Err(error) => {
                            if let Ok(job) = receiver.recv() {
                                let _ = job
                                    .reply
                                    .send(Err(RecoverySnapshotPager::unavailable("open", error)));
                            }
                            return;
                        }
                    };
                if let Err(error) = connection.execute_batch("BEGIN DEFERRED TRANSACTION") {
                    if let Ok(job) = receiver.recv() {
                        let _ = job
                            .reply
                            .send(Err(RecoverySnapshotPager::unavailable("begin", error)));
                    }
                    return;
                }
                while let Ok(job) = receiver.recv() {
                    let result = run_query_in_recovery_snapshot(
                        &connection,
                        &context,
                        &job.query,
                        Some(&worker_snapshot_id),
                    );
                    let retain = RecoverySnapshotPager::should_retain(&result);
                    let _ = job.reply.send(result);
                    if !retain {
                        break;
                    }
                }
                let _ = connection.execute_batch("ROLLBACK");
            }) {
            Ok(worker) => worker,
            Err(error) => {
                self.remove(&snapshot_id);
                return Err(Self::unavailable("spawn", error));
            }
        };
        self.workers
            .lock()
            .expect("recovery snapshot worker list poisoned")
            .push(worker);
        self.dispatch(&snapshot_id, &sender, query).await
    }

    pub fn close(&self) {
        {
            let mut state = self.state.lock().expect("recovery snapshot pager poisoned");
            state.closed = true;
            state.active.clear();
            state.issue_order.clear();
        }
        let workers = self
            .workers
            .lock()
            .expect("recovery snapshot worker list poisoned")
            .drain(..)
            .collect::<Vec<_>>();
        for worker in workers {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod canonical_runtime_owner_snapshot_tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionOrigin, NodeCompletion, NodeDefinition, NodeFact, NodeKind, SessionRootFact,
        SessionSpec, StartedFact, TreeRootFact, WorkflowDefinition, WorkflowRootFact,
    };

    fn connection_with_node_events() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory SQLite");
        connection
            .execute_batch(
                "CREATE TABLE node_events (
                    tree_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    node_execution_id TEXT NOT NULL,
                    parent_id TEXT,
                    node_name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    attempt INTEGER NOT NULL,
                    event_type TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    PRIMARY KEY (tree_id, seq)
                );",
            )
            .expect("node_events table");
        connection
    }

    fn insert_root(connection: &Connection, tree_id: &str, fact: &NodeFact) {
        connection
            .execute(
                "INSERT INTO node_events (
                    tree_id, seq, node_execution_id, parent_id, node_name, kind,
                    attempt, event_type, detail, timestamp
                 ) VALUES (?1, 1, ?1, NULL, 'main', 'session', 1, ?2, ?3, 1)",
                params![tree_id, fact.event_type(), fact.encode_detail().unwrap()],
            )
            .expect("insert root fact");
    }

    fn workflow_root(worktree_path: &str) -> NodeFact {
        NodeFact::Started(StartedFact {
            parent: None,
            root: Some(TreeRootFact::Workflow(WorkflowRootFact {
                workflow_name: "wf".to_string(),
                worktree_path: worktree_path.to_string(),
                created_from: ExecutionOrigin::Cli,
                request: String::new(),
                definition: WorkflowDefinition {
                    name: "wf".to_string(),
                    description: String::new(),
                    builtin: false,
                    schemas: Default::default(),
                    nodes: vec![NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Session(SessionSpec::default()),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::Auto,
                        worktree: None,
                    }],
                    entry: "main".to_string(),
                },
            })),
        })
    }

    fn connection_with_active_workflow_owners(count: usize) -> Connection {
        let connection = connection_with_node_events();
        for index in 0..count {
            insert_root(
                &connection,
                &format!("execution-{index}"),
                &workflow_root(&format!("/snapshot/worktree-{index}")),
            );
        }
        connection
    }

    #[test]
    fn app_data_gc_owner_snapshot_returns_one_bounded_lightweight_inventory() {
        let connection = connection_with_active_workflow_owners(2);

        let owners =
            canonical_runtime_owner_snapshot(&connection, 2).expect("complete owner snapshot");

        assert_eq!(owners.len(), 2);
        assert!(owners
            .iter()
            .all(|owner| matches!(owner, CanonicalRuntimeOwnerView::ActiveWorkflow { .. })));
    }

    #[test]
    fn app_data_gc_owner_snapshot_lists_open_session_trees() {
        let connection = connection_with_node_events();
        insert_root(
            &connection,
            "agent-session-1",
            &NodeFact::Started(StartedFact {
                parent: None,
                root: Some(TreeRootFact::Session(SessionRootFact {
                    workspace_identity: "/snapshot/worktree-a".to_string(),
                    worktree_path: "/snapshot/worktree-a".to_string(),
                    session: SessionSpec::default(),
                    created_from: ExecutionOrigin::DesktopUi,
                })),
            }),
        );

        let owners =
            canonical_runtime_owner_snapshot(&connection, 8).expect("complete owner snapshot");

        assert_eq!(
            owners,
            vec![CanonicalRuntimeOwnerView::AgentSession {
                worktree_path: "/snapshot/worktree-a".to_string(),
                active: true,
            }]
        );
    }

    #[test]
    fn app_data_gc_owner_snapshot_limit_plus_one_fails_closed() {
        let connection = connection_with_active_workflow_owners(2);

        assert_eq!(
            canonical_runtime_owner_snapshot(&connection, 1),
            Err(LocalEventQueryError::ResponseTooLarge)
        );
    }
}
