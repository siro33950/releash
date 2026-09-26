//! The `commit_batch` transaction (design "Commit transaction", 9 steps).
//!
//! Executed on the writer thread only. Any failure before SQLite COMMIT
//! rolls back to the pre-batch state; any error or reply loss between the
//! start of COMMIT and the completed fresh readback is `OutcomeUnknown` for
//! the same commit identity.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::indexed_projection_codec::indexed_session_public_columns;
use crate::adaptor::gateway::local_event_store::node_events;
use crate::adaptor::gateway::local_event_store::projection_record_codec::encode_session_projection_update_v1;
use crate::adaptor::gateway::local_event_store::writer::{PreparedBatch, PreparedEvent};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitResolution, CommittedBatch, CommittedStreamHead,
    IdempotencyBinding, LocalEventQueryError, LocalStateMutation, RevisionGuard,
    SafeOperationFailure, SessionOperationFailureKind, SessionProjectionMutation, StreamId,
    StreamVersion,
};

pub(crate) const SQL_SEAL_EVENT_COUNT: &str = "SELECT COUNT(*) FROM events
     WHERE global_sequence BETWEEN ?1 AND ?2 AND commit_id = ?3";

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(super) fn storage_unavailable(error: &rusqlite::Error) -> CommitBatchError {
    let correlation = correlation_id();
    log::warn!("local event store sqlite failure [{correlation}]: {error}");
    use crate::adaptor::gateway::shared::sqlite_failure::{condition, SqliteFailureCondition};
    use crate::domain::failure::TechnicalFailureNature;
    let condition = condition(error);
    if condition == SqliteFailureCondition::Corrupt {
        return CommitBatchError::Corrupt {
            correlation_id: correlation,
        };
    }
    let failure = SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        if condition == SqliteFailureCondition::Busy {
            TechnicalFailureNature::Transient
        } else {
            TechnicalFailureNature::Other
        },
        "local event store write failed",
        correlation,
    );
    if condition == SqliteFailureCondition::Inaccessible {
        CommitBatchError::StorageAccessRequired { failure }
    } else {
        CommitBatchError::StorageUnavailable { failure }
    }
}

fn corrupt(context: &str) -> CommitBatchError {
    let correlation = correlation_id();
    log::error!("local event store corrupt state [{correlation}]: {context}");
    CommitBatchError::Corrupt {
        correlation_id: correlation,
    }
}

fn conflict(current: i64) -> CommitBatchError {
    CommitBatchError::StreamHeadConflict {
        current: StreamVersion::new(current.max(0)).unwrap_or(StreamVersion::zero()),
    }
}

fn check_guard(existing: Option<i64>, guard: RevisionGuard) -> Result<(), CommitBatchError> {
    match (existing, guard) {
        (None, RevisionGuard::Absent) => Ok(()),
        (Some(revision), RevisionGuard::Expected(expected)) if revision == expected.value() => {
            Ok(())
        }
        (Some(revision), _) => Err(conflict(revision)),
        (None, RevisionGuard::Expected(_)) => Err(conflict(0)),
    }
}

fn encode_stream_heads(heads: &[CommittedStreamHead]) -> String {
    let entries: Vec<(String, i64)> = heads
        .iter()
        .map(|head| (head.stream_id.as_str().to_string(), head.head.value()))
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

fn decode_stream_heads(raw: &str) -> Result<Vec<CommittedStreamHead>, CommitBatchError> {
    let entries: Vec<(String, i64)> =
        serde_json::from_str(raw).map_err(|_| corrupt("stream_heads_json parse failed"))?;
    entries
        .into_iter()
        .map(|(stream_id, head)| {
            Ok(CommittedStreamHead {
                stream_id: StreamId::parse(&stream_id)
                    .map_err(|_| corrupt("stream_heads_json stream id invalid"))?,
                head: StreamVersion::new(head)
                    .map_err(|_| corrupt("stream_heads_json head invalid"))?,
            })
        })
        .collect()
}

struct SealedCommitRow {
    commit_id: String,
    payload_hash: Vec<u8>,
    state: String,
    first_global_sequence: Option<i64>,
    last_global_sequence: Option<i64>,
    event_count: i64,
    mutation_count: i64,
    stream_heads_json: String,
    result_hash: Option<Vec<u8>>,
}

fn lookup_idempotency(
    connection: &Connection,
    idempotency: &IdempotencyBinding,
) -> Result<Option<SealedCommitRow>, rusqlite::Error> {
    connection
        .query_row(
            "SELECT commit_id, payload_hash, state, first_global_sequence, last_global_sequence,
                    event_count, mutation_count, stream_heads_json, result_hash
             FROM logical_commits
             WHERE installation_id = ?1 AND operation_kind = ?2 AND idempotency_key = ?3",
            params![
                idempotency.installation_id,
                idempotency.operation_kind.label(),
                idempotency.idempotency_key
            ],
            |row| {
                Ok(SealedCommitRow {
                    commit_id: row.get(0)?,
                    payload_hash: row.get(1)?,
                    state: row.get(2)?,
                    first_global_sequence: row.get(3)?,
                    last_global_sequence: row.get(4)?,
                    event_count: row.get(5)?,
                    mutation_count: row.get(6)?,
                    stream_heads_json: row.get(7)?,
                    result_hash: row.get(8)?,
                })
            },
        )
        .optional()
}

fn committed_batch_from_row(
    prepared_commit_id: &crate::domain::local_event::CommitIdentity,
    row: &SealedCommitRow,
) -> Result<CommittedBatch, CommitBatchError> {
    use crate::domain::local_event::GlobalSequence;
    if row.state != "sealed" {
        return Err(corrupt("visible logical commit is not sealed"));
    }
    let result_hash_bytes = row
        .result_hash
        .as_ref()
        .ok_or_else(|| corrupt("sealed commit without result hash"))?;
    let result_hash: [u8; 32] = result_hash_bytes
        .as_slice()
        .try_into()
        .map_err(|_| corrupt("sealed commit result hash length"))?;
    let sequence_range = match (row.first_global_sequence, row.last_global_sequence) {
        (Some(first), Some(last)) => Some((
            GlobalSequence::new(first).map_err(|_| corrupt("sealed commit first sequence"))?,
            GlobalSequence::new(last).map_err(|_| corrupt("sealed commit last sequence"))?,
        )),
        (None, None) => None,
        _ => return Err(corrupt("sealed commit half-open sequence range")),
    };
    Ok(CommittedBatch {
        commit_id: prepared_commit_id.clone(),
        sequence_range,
        stream_heads: decode_stream_heads(&row.stream_heads_json)?,
        event_count: row.event_count,
        mutation_count: row.mutation_count,
        result_hash,
    })
}

fn result_hash_of(
    commit_id: &str,
    sequence_range: Option<(i64, i64)>,
    event_ids: &[String],
    heads: &[CommittedStreamHead],
    mutation_count: i64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(commit_id.as_bytes());
    if let Some((first, last)) = sequence_range {
        hasher.update(first.to_be_bytes());
        hasher.update(last.to_be_bytes());
    }
    for event_id in event_ids {
        hasher.update((event_id.len() as u64).to_be_bytes());
        hasher.update(event_id.as_bytes());
    }
    for head in heads {
        hasher.update(head.stream_id.as_str().as_bytes());
        hasher.update(head.head.value().to_be_bytes());
    }
    hasher.update(mutation_count.to_be_bytes());
    hasher.finalize().into()
}

/// Resolve a commit identity to its sealed result or proven absence.
/// Used by `resolve_commit` and the `CommitByIdentity` query.
pub(crate) fn resolve_commit_row(
    connection: &Connection,
    commit_id: &crate::domain::local_event::CommitIdentity,
) -> Result<CommitResolution, LocalEventQueryError> {
    let row = connection
        .query_row(
            "SELECT commit_id, payload_hash, state, first_global_sequence, last_global_sequence,
                    event_count, mutation_count, stream_heads_json, result_hash
             FROM logical_commits WHERE commit_id = ?1",
            params![commit_id.as_str()],
            |row| {
                Ok(SealedCommitRow {
                    commit_id: row.get(0)?,
                    payload_hash: row.get(1)?,
                    state: row.get(2)?,
                    first_global_sequence: row.get(3)?,
                    last_global_sequence: row.get(4)?,
                    event_count: row.get(5)?,
                    mutation_count: row.get(6)?,
                    stream_heads_json: row.get(7)?,
                    result_hash: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|error| super::reader::storage_unavailable(&error))?;
    match row {
        None => Ok(CommitResolution::NotCommitted),
        Some(row) => {
            let batch = committed_batch_from_row(commit_id, &row).map_err(|error| match error {
                CommitBatchError::Corrupt { correlation_id } => {
                    LocalEventQueryError::Corrupt { correlation_id }
                }
                _ => LocalEventQueryError::Internal {
                    correlation_id: correlation_id(),
                },
            })?;
            Ok(CommitResolution::Committed(batch))
        }
    }
}

/// Execute one prepared batch on the writer connection.
///
/// Returns the commit result. The reply-drop fault is handled by the caller;
/// this function only distinguishes rollback errors from `OutcomeUnknown`.
pub fn execute_commit(
    connection: &Connection,
    prepared: &PreparedBatch,
    now_ms: i64,
    fault: &FaultInjector,
) -> Result<CommitBatchResult, CommitBatchError> {
    let batch = &prepared.batch;
    let commit_id = batch.commit_id.as_str().to_string();

    // Step 1 (batch shape) was validated before queue admission.
    if fault.take_fail_before_begin() {
        return Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                crate::domain::failure::TechnicalFailureNature::Transient,
                "injected failure before transaction begin",
                correlation_id(),
            ),
        });
    }

    // Step 2: BEGIN IMMEDIATE.
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| storage_unavailable(&error))?;

    let outcome = match execute_in_transaction(connection, prepared, now_ms, fault) {
        Ok(outcome) => outcome,
        Err(error @ CommitBatchError::OutcomeUnknown { .. }) => {
            // COMMIT already started; do not roll back.
            return Err(error);
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    };

    // Step 8 second half: fresh readback with a separate statement.
    if fault.take_crash_after_commit_before_readback() {
        return Err(CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        });
    }
    let readback = lookup_idempotency(connection, &batch.idempotency).map_err(|_| {
        CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        }
    })?;
    let Some(row) = readback else {
        return Err(CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        });
    };
    if row.commit_id != commit_id {
        return Err(corrupt("readback returned a different commit identity"));
    }
    let committed = committed_batch_from_row(&batch.commit_id, &row)?;
    match outcome {
        CommitOutcome::Committed => Ok(CommitBatchResult::Committed(committed)),
        CommitOutcome::Replayed => Ok(CommitBatchResult::Replayed(committed)),
    }
}

enum CommitOutcome {
    Committed,
    Replayed,
}

fn execute_in_transaction(
    connection: &Connection,
    prepared: &PreparedBatch,
    now_ms: i64,
    fault: &FaultInjector,
) -> Result<CommitOutcome, CommitBatchError> {
    let batch = &prepared.batch;
    let commit_id = batch.commit_id.as_str();

    // Step 3: idempotency point lookup.
    if let Some(row) = lookup_idempotency(connection, &batch.idempotency)
        .map_err(|error| storage_unavailable(&error))?
    {
        if row.state != "sealed" {
            return Err(corrupt("existing logical commit is not sealed"));
        }
        let payload_matches =
            row.payload_hash.as_slice() == batch.idempotency.payload_hash.as_slice();
        if !payload_matches || row.commit_id != commit_id {
            return Err(CommitBatchError::PayloadConflict);
        }
        // Same binding: the transaction commits nothing; COMMIT below is a
        // no-op and the readback returns the saved result.
        connection
            .execute_batch("COMMIT")
            .map_err(|_| CommitBatchError::OutcomeUnknown {
                identity: batch.commit_id.clone(),
            })?;
        return Ok(CommitOutcome::Replayed);
    }
    // The commit identity itself must also be new.
    let existing_commit: Option<String> = connection
        .query_row(
            "SELECT commit_id FROM logical_commits WHERE commit_id = ?1",
            params![commit_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    if existing_commit.is_some() {
        return Err(CommitBatchError::PayloadConflict);
    }
    for event in &prepared.node_events {
        if event.expect_tree_absent
            && node_events::first_row_of_tree(connection, &event.row.tree_id)
                .map_err(|error| storage_unavailable(&error))?
                .is_some()
        {
            return Err(CommitBatchError::PayloadConflict);
        }
    }

    // Step 4: expected stream heads and mutation revision guards.
    let mut current_heads: Vec<(StreamId, i64)> = Vec::with_capacity(batch.expected_heads.len());
    for expected in &batch.expected_heads {
        let current: Option<i64> = connection
            .query_row(
                "SELECT head FROM stream_heads WHERE stream_id = ?1",
                params![expected.stream_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_unavailable(&error))?;
        let current = current.unwrap_or(0);
        if current != expected.expected.value() {
            return Err(conflict(current));
        }
        current_heads.push((expected.stream_id.clone(), current));
    }
    validate_mutation_guards(connection, &batch.state_mutations)?;

    // Sequence allocation bounds (fail typed before overflow).
    let next_global: i64 = connection
        .query_row(
            "SELECT next_global_sequence FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_unavailable(&error))?;
    let event_count = prepared.events.len() as i64;
    if event_count > 0 && next_global.checked_add(event_count - 1).is_none() {
        return Err(CommitBatchError::SequenceExhausted);
    }
    for (stream_id, current) in &current_heads {
        let events_for_stream = prepared
            .events
            .iter()
            .filter(|event| &event.stream_id == stream_id)
            .count() as i64;
        if current.checked_add(events_for_stream).is_none() {
            return Err(CommitBatchError::SequenceExhausted);
        }
    }

    // Step 5: insert the preparing logical commit, then events with
    // contiguous global / stream sequences.
    let mut new_heads: Vec<CommittedStreamHead> = Vec::with_capacity(current_heads.len());
    for (stream_id, current) in &current_heads {
        let events_for_stream = prepared
            .events
            .iter()
            .filter(|event| &event.stream_id == stream_id)
            .count() as i64;
        new_heads.push(CommittedStreamHead {
            stream_id: stream_id.clone(),
            head: StreamVersion::new(current + events_for_stream)
                .map_err(|_| CommitBatchError::SequenceExhausted)?,
        });
    }
    connection
        .execute(
            "INSERT INTO logical_commits (
                commit_id, installation_id, operation_kind, idempotency_key, payload_hash,
                state, first_global_sequence, last_global_sequence, event_count,
                mutation_count, stream_heads_json, result_hash, committed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'preparing', NULL, NULL, ?6, ?7, ?8, NULL, ?9)",
            params![
                commit_id,
                batch.idempotency.installation_id,
                batch.idempotency.operation_kind.label(),
                batch.idempotency.idempotency_key,
                batch.idempotency.payload_hash.as_slice(),
                event_count,
                batch.state_mutations.len() as i64,
                encode_stream_heads(&new_heads),
                now_ms
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;

    let mut event_ids: Vec<String> = Vec::with_capacity(prepared.events.len());
    let mut stream_positions: Vec<(StreamId, i64)> = current_heads.clone();
    for (offset, event) in prepared.events.iter().enumerate() {
        let global = next_global + offset as i64;
        let position = stream_positions
            .iter_mut()
            .find(|(stream_id, _)| stream_id == &event.stream_id)
            .ok_or_else(|| corrupt("event stream missing from expected heads"))?;
        position.1 += 1;
        let event_id = format!("{commit_id}.{global}");
        insert_event(connection, commit_id, &event_id, event, position.1, global)?;
        event_ids.push(event_id);
        fail_after_participant_write_if_armed(fault)?;
    }
    for head in &new_heads {
        connection
            .execute(
                "INSERT INTO stream_heads (stream_id, head, updated_commit_id)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (stream_id) DO UPDATE SET
                     head = excluded.head,
                     updated_commit_id = excluded.updated_commit_id",
                params![head.stream_id.as_str(), head.head.value(), commit_id],
            )
            .map_err(|error| storage_unavailable(&error))?;
    }

    // Step 6: state mutations, direct indexes, projections.
    for mutation in &batch.state_mutations {
        apply_mutation(connection, commit_id, mutation)?;
        fail_after_participant_write_if_armed(fault)?;
    }
    for event in &prepared.node_events {
        node_events::append_node_event(connection, &event.row, event.timestamp_ms)
            .map_err(|error| storage_unavailable(&error))?;
        fail_after_participant_write_if_armed(fault)?;
    }

    // Step 7: verify counts / range and seal.
    let sequence_range = if event_count > 0 {
        Some((next_global, next_global + event_count - 1))
    } else {
        None
    };
    let stored_event_count: i64 = match sequence_range {
        Some((first, last)) => connection
            .query_row(
                SQL_SEAL_EVENT_COUNT,
                params![first, last, commit_id],
                |row| row.get(0),
            )
            .map_err(|error| storage_unavailable(&error))?,
        None => 0,
    };
    if stored_event_count != event_count {
        return Err(corrupt("event count mismatch at seal"));
    }
    let result_hash = result_hash_of(
        commit_id,
        sequence_range,
        &event_ids,
        &new_heads,
        batch.state_mutations.len() as i64,
    );
    connection
        .execute(
            "UPDATE logical_commits SET
                state = 'sealed',
                first_global_sequence = ?2,
                last_global_sequence = ?3,
                result_hash = ?4
             WHERE commit_id = ?1",
            params![
                commit_id,
                sequence_range.map(|range| range.0),
                sequence_range.map(|range| range.1),
                result_hash.as_slice()
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;
    if event_count > 0 {
        connection
            .execute(
                "UPDATE store_metadata SET next_global_sequence = ?1 WHERE id = 1",
                params![next_global + event_count],
            )
            .map_err(|error| storage_unavailable(&error))?;
    }

    if fault.take_fail_before_commit() {
        return Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::StorageUnavailable,
                crate::domain::failure::TechnicalFailureNature::Transient,
                "injected failure before COMMIT",
                correlation_id(),
            ),
        });
    }

    // Step 8 first half: COMMIT. From here every failure is OutcomeUnknown.
    connection
        .execute_batch("COMMIT")
        .map_err(|_| CommitBatchError::OutcomeUnknown {
            identity: batch.commit_id.clone(),
        })?;
    Ok(CommitOutcome::Committed)
}

fn fail_after_participant_write_if_armed(fault: &FaultInjector) -> Result<(), CommitBatchError> {
    if !fault.take_fail_after_participant_write() {
        return Ok(());
    }
    Err(CommitBatchError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            crate::domain::failure::TechnicalFailureNature::Transient,
            "injected failure after participant write",
            correlation_id(),
        ),
    })
}

fn insert_event(
    connection: &Connection,
    commit_id: &str,
    event_id: &str,
    event: &PreparedEvent,
    stream_sequence: i64,
    global_sequence: i64,
) -> Result<(), CommitBatchError> {
    connection
        .execute(
            "INSERT INTO events (
                global_sequence, event_id, commit_id, stream_id, stream_sequence,
                event_type, payload_version, occurred_at, payload, payload_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                global_sequence,
                event_id,
                commit_id,
                event.stream_id.as_str(),
                stream_sequence,
                event.payload.event_type,
                event.payload.payload_version,
                event.occurred_at_ms.to_string(),
                event.payload.payload,
                event.payload_sha256.as_slice()
            ],
        )
        .map_err(|error| storage_unavailable(&error))?;
    Ok(())
}

// --- Mutation guard validation (step 4) ---

fn validate_mutation_guards(
    connection: &Connection,
    mutations: &[LocalStateMutation],
) -> Result<(), CommitBatchError> {
    for mutation in mutations {
        validate_one_guard(connection, mutation)?;
    }
    Ok(())
}

fn read_revision(
    connection: &Connection,
    sql: &str,
    parameters: impl rusqlite::Params,
) -> Result<Option<i64>, CommitBatchError> {
    connection
        .query_row(sql, parameters, |row| row.get::<_, i64>(0))
        .optional()
        .map_err(|error| storage_unavailable(&error))
}

fn validate_one_guard(
    connection: &Connection,
    mutation: &LocalStateMutation,
) -> Result<(), CommitBatchError> {
    match mutation {
        LocalStateMutation::SessionProjection(m) => {
            indexed_session_public_columns(&m.projection)
                .map_err(|_| CommitBatchError::PayloadConflict)?;
            let existing = read_revision(
                connection,
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![m.session_id],
            )?;
            check_guard(existing, m.expected)
        }
        LocalStateMutation::AgentSessionRemoval(m) => {
            if m.node_event_tree_id.trim().is_empty() {
                return Err(CommitBatchError::PayloadConflict);
            }
            match (
                &m.ownership_projection_id,
                &m.ownership_stream,
                m.ownership_expected,
            ) {
                (Some(projection_id), Some(stream), Some(expected)) => {
                    let projection_revision = read_revision(
                        connection,
                        "SELECT revision FROM session_projection WHERE session_id = ?1",
                        params![projection_id],
                    )?;
                    check_guard(projection_revision, RevisionGuard::Expected(expected))?;
                    let stream_head = read_revision(
                        connection,
                        "SELECT head FROM stream_heads WHERE stream_id = ?1",
                        params![stream.as_str()],
                    )?;
                    check_guard(stream_head, RevisionGuard::Expected(expected))
                }
                (None, None, None) => Ok(()),
                _ => Err(CommitBatchError::PayloadConflict),
            }
        }
    }
}

fn apply_mutation(
    connection: &Connection,
    commit_id: &str,
    mutation: &LocalStateMutation,
) -> Result<(), CommitBatchError> {
    let run = |result: Result<usize, rusqlite::Error>| -> Result<(), CommitBatchError> {
        result
            .map(|_| ())
            .map_err(|error| storage_unavailable(&error))
    };
    match mutation {
        LocalStateMutation::SessionProjection(m) => {
            apply_session_projection(connection, commit_id, m)
        }
        LocalStateMutation::AgentSessionRemoval(m) => {
            run(node_events::delete_tree(connection, &m.node_event_tree_id))?;
            if let (Some(projection_id), Some(stream), Some(_)) = (
                &m.ownership_projection_id,
                &m.ownership_stream,
                m.ownership_expected,
            ) {
                run(connection.execute(
                    "DELETE FROM session_projection WHERE session_id = ?1",
                    params![projection_id],
                ))?;
                run(connection.execute(
                    "DELETE FROM events WHERE stream_id = ?1",
                    params![stream.as_str()],
                ))?;
                run(connection.execute(
                    "DELETE FROM stream_heads WHERE stream_id = ?1",
                    params![stream.as_str()],
                ))?;
            }
            Ok(())
        }
    }
}

fn run(result: Result<usize, rusqlite::Error>) -> Result<(), CommitBatchError> {
    result
        .map(|_| ())
        .map_err(|error| storage_unavailable(&error))
}

fn apply_session_projection(
    connection: &Connection,
    commit_id: &str,
    m: &SessionProjectionMutation,
) -> Result<(), CommitBatchError> {
    let existing = connection
        .query_row(
            "SELECT projection FROM session_projection WHERE session_id = ?1",
            params![m.session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| storage_unavailable(&error))?;
    let encoded =
        encode_session_projection_update_v1(existing.as_deref(), &m.projection, &m.session_id)
            .map_err(|_| CommitBatchError::PayloadConflict)?;
    let public = indexed_session_public_columns(&m.projection)
        .map_err(|_| CommitBatchError::PayloadConflict)?;
    run(connection.execute(
        "INSERT INTO session_projection
            (session_id, projection, revision, commit_id, workspace_identity,
             public_list_kind, public_sort_key_bits, public_summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (session_id) DO UPDATE SET
            projection = excluded.projection,
            revision = excluded.revision,
            commit_id = excluded.commit_id,
            workspace_identity = excluded.workspace_identity,
            public_list_kind = excluded.public_list_kind,
            public_sort_key_bits = excluded.public_sort_key_bits,
            public_summary = excluded.public_summary",
        params![
            m.session_id,
            encoded,
            m.revision.value(),
            commit_id,
            public.workspace_identity,
            public.list_kind,
            public.sort_key_bits,
            public.summary
        ],
    ))
}

#[cfg(test)]
#[path = "commit_test.rs"]
mod commit_tests;
