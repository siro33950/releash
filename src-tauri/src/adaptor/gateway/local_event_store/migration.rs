//! One-shot legacy source inventory and raw-preserving staging import.
//!
//! The migration runs while the exclusive writer lock is held and before
//! normal admission opens. Source files are never modified or deleted.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

use super::authority::{
    cas_authority, AuthorityError, AuthorityMigrationRef, LocalStoreAuthorityPointerV1, StoreLayout,
};
use super::envelope::EventCodecRegistry;
use super::projection_record_codec::{
    decode_message_projection_record_v1, decode_session_projection_record_v1,
};
use crate::adaptor::gateway::bounded_json::{stream_json_array, stream_ndjson_records};
use crate::domain::local_event::{LocalDomainEvent, StreamId};

const LEGACY_STATE_RECORD_MAX_BYTES: usize = 16 * 1024 * 1024;

fn validate_legacy_state_record_bound(raw: &str, label: &str) -> Result<(), rusqlite::Error> {
    if raw.is_empty() || raw.len() > LEGACY_STATE_RECORD_MAX_BYTES {
        return Err(migration_collision(label));
    }
    Ok(())
}
use crate::usecase::agent_session::session::AgentSessionProjectionCodec;

const SOURCE_ROOTS: &[&str] = &[
    "sessions",
    "workflow_execution_logs",
    "workflow_executions",
    "workflow_event_logs",
    "session_titles.json",
];
const IMPORT_RECORD_LIMIT: usize = 256;
const IMPORT_BYTE_LIMIT: usize = 16 * 1024 * 1024;
const RAW_CHUNK_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
struct SourceRecord {
    relative_path: String,
    size: u64,
    modified_ms: u128,
    sha256: [u8; 32],
}

fn source_modified_ms(metadata: &std::fs::Metadata) -> Result<u128, std::io::Error> {
    Ok(metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis())
}

fn hash_source_with_checkpoint<F>(
    path: &Path,
    at_checkpoint: &mut F,
) -> Result<[u8; 32], std::io::Error>
where
    F: FnMut(),
{
    let mut source = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        at_checkpoint();
    }
    Ok(hasher.finalize().into())
}

/// Re-read exactly one inventoried source. The legacy codecs are record
/// codecs, not streaming codecs, so an over-sized source is rejected before
/// allocation rather than silently widening the migration transaction.
fn read_inventoried_source(
    app_data_root: &Path,
    record: &SourceRecord,
) -> Result<Vec<u8>, std::io::Error> {
    if record.size > IMPORT_BYTE_LIMIT as u64 {
        return Err(std::io::Error::other(
            "one legacy semantic source exceeds the 16 MiB import bound",
        ));
    }
    let path = app_data_root.join(&record.relative_path);
    let before = std::fs::metadata(&path)?;
    if before.len() != record.size || source_modified_ms(&before)? != record.modified_ms {
        return Err(std::io::Error::other(
            "legacy source changed after migration inventory",
        ));
    }
    let source = std::fs::File::open(&path)?;
    let capacity = usize::try_from(record.size)
        .map_err(|_| std::io::Error::other("legacy source size is not addressable"))?;
    let mut raw = Vec::with_capacity(capacity);
    source
        .take((IMPORT_BYTE_LIMIT + 1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > IMPORT_BYTE_LIMIT || raw.len() as u64 != record.size {
        return Err(std::io::Error::other(
            "legacy source changed or exceeds the 16 MiB import bound",
        ));
    }
    let after = std::fs::metadata(&path)?;
    if after.len() != record.size
        || source_modified_ms(&after)? != record.modified_ms
        || <[u8; 32]>::from(Sha256::digest(&raw)) != record.sha256
    {
        return Err(std::io::Error::other(
            "legacy source changed after migration inventory",
        ));
    }
    Ok(raw)
}

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes_read: u64,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        self.bytes_read = self.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
}

fn stream_inventoried_json_array<F>(
    app_data_root: &Path,
    record: &SourceRecord,
    visit: F,
) -> Result<u64, String>
where
    F: FnMut(u64, &[u8]) -> Result<(), String>,
{
    let path = app_data_root.join(&record.relative_path);
    let before = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    if before.len() != record.size
        || source_modified_ms(&before).map_err(|error| error.to_string())? != record.modified_ms
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    let source = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut reader = HashingReader {
        inner: source,
        hasher: Sha256::new(),
        bytes_read: 0,
    };
    let (buffered, count) =
        stream_json_array(std::io::BufReader::new(reader), IMPORT_BYTE_LIMIT, visit)?;
    reader = buffered.into_inner();
    let after = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    let digest: [u8; 32] = reader.hasher.finalize().into();
    if reader.bytes_read != record.size
        || after.len() != record.size
        || source_modified_ms(&after).map_err(|error| error.to_string())? != record.modified_ms
        || digest != record.sha256
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    Ok(count)
}

fn decode_inventoried_large_message(
    app_data_root: &Path,
    record: &SourceRecord,
) -> Result<
    crate::adaptor::gateway::agent_session::session_storage::LegacySessionProjectionV1,
    String,
> {
    let path = app_data_root.join(&record.relative_path);
    let before = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    if before.len() != record.size
        || source_modified_ms(&before).map_err(|error| error.to_string())? != record.modified_ms
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    let source = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut reader = HashingReader {
        inner: source,
        hasher: Sha256::new(),
        bytes_read: 0,
    };
    let decoded =
        crate::adaptor::gateway::agent_session::session_storage::decode_streaming_legacy_message_projection_v1(
            &record.relative_path,
            &mut reader,
        )?
        .ok_or_else(|| "oversized legacy message path is incompatible".to_string())?;
    let after = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    let digest: [u8; 32] = reader.hasher.finalize().into();
    if reader.bytes_read != record.size
        || after.len() != record.size
        || source_modified_ms(&after).map_err(|error| error.to_string())? != record.modified_ms
        || digest != record.sha256
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    Ok(decoded)
}

fn stream_inventoried_ndjson<F>(
    app_data_root: &Path,
    record: &SourceRecord,
    visit: F,
) -> Result<u64, String>
where
    F: FnMut(u64, u64, &[u8]) -> Result<(), String>,
{
    let path = app_data_root.join(&record.relative_path);
    let before = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    if before.len() != record.size
        || source_modified_ms(&before).map_err(|error| error.to_string())? != record.modified_ms
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    let source = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let hashing = HashingReader {
        inner: source,
        hasher: Sha256::new(),
        bytes_read: 0,
    };
    let (reader, record_ordinal) =
        stream_ndjson_records(std::io::BufReader::new(hashing), IMPORT_BYTE_LIMIT, visit)?;
    let reader = reader.into_inner();
    let after = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    let digest: [u8; 32] = reader.hasher.finalize().into();
    if reader.bytes_read != record.size
        || after.len() != record.size
        || source_modified_ms(&after).map_err(|error| error.to_string())? != record.modified_ms
        || digest != record.sha256
    {
        return Err("legacy source changed after migration inventory".to_string());
    }
    Ok(record_ordinal)
}

fn chunk_checkpoint_value(
    transaction: &Transaction<'_>,
    migration_id: &str,
) -> Result<serde_json::Value, rusqlite::Error> {
    let checkpoint: String = transaction.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy raw chunk checkpoint is invalid"))
}

fn has_record_streaming_codec(relative_path: &str) -> Result<bool, String> {
    if crate::adaptor::gateway::agent_session::session_storage::legacy_agent_event_source_identity_v1(
        relative_path,
    )?
    .is_some()
        || crate::adaptor::gateway::agent_session::session_storage::legacy_message_source_identity_v1(
            relative_path,
        )?
        .is_some()
    {
        return Ok(true);
    }
    Ok(
        crate::adaptor::gateway::workflow::log::legacy_workflow_event_source_identity_v1(
            relative_path,
        )?
        .is_some(),
    )
}

#[allow(clippy::too_many_arguments)] // Migration checkpoint participants stay explicit.
fn import_chunked_raw_source<F>(
    connection: &mut Connection,
    app_data_root: &Path,
    record: &SourceRecord,
    migration_id: &str,
    commit_id: &str,
    source_ordinal: usize,
    imported_raw_record_count: i64,
    at_checkpoint: &mut F,
) -> Result<(), rusqlite::Error>
where
    F: FnMut(&Connection),
{
    if !has_record_streaming_codec(&record.relative_path)
        .map_err(|reason| migration_collision(&reason))?
    {
        return Err(migration_collision(
            "oversized legacy source has no record-streaming codec",
        ));
    }
    let source_ordinal_i64 = i64::try_from(source_ordinal)
        .map_err(|_| migration_collision("legacy source ordinal overflow"))?;
    let inserted_parent = {
        let transaction = connection.transaction()?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO legacy_raw_records
                (migration_id, source_ordinal, source_path, source_size,
                 modified_ms, record_count, raw, raw_sha256)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, X'', ?6)",
            params![
                migration_id,
                source_ordinal_i64,
                record.relative_path,
                record.size.to_string(),
                record.modified_ms.to_string(),
                record.sha256.as_slice(),
            ],
        )?;
        if inserted == 1 {
            let mut checkpoint = chunk_checkpoint_value(&transaction, migration_id)?;
            let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                migration_collision("legacy raw chunk checkpoint is not an object")
            })?;
            checkpoint.insert(
                "substep".to_string(),
                serde_json::Value::String("raw_chunks".to_string()),
            );
            checkpoint.insert(
                "source_ordinal".to_string(),
                serde_json::Value::from(source_ordinal),
            );
            checkpoint.insert(
                "source_record_ordinal".to_string(),
                serde_json::Value::from(0),
            );
            checkpoint.insert("source_byte_offset".to_string(), serde_json::Value::from(0));
            checkpoint.insert(
                "next_source_ordinal".to_string(),
                serde_json::Value::from(source_ordinal),
            );
            transaction.execute(
                "UPDATE local_store_migrations
                 SET checkpoint = ?2, revision = revision + 1, commit_id = ?3
                 WHERE migration_id = ?1",
                params![
                    migration_id,
                    serde_json::Value::Object(checkpoint.clone()).to_string(),
                    commit_id,
                ],
            )?;
        }
        transaction.commit()?;
        inserted == 1
    };
    if inserted_parent {
        at_checkpoint(connection);
    }

    let parent: (String, String, String, i64, Vec<u8>, Vec<u8>) = connection.query_row(
        "SELECT source_path, source_size, modified_ms, record_count, raw, raw_sha256
         FROM legacy_raw_records
         WHERE migration_id = ?1 AND source_ordinal = ?2",
        params![migration_id, source_ordinal_i64],
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
    )?;
    if parent.0 != record.relative_path
        || parent.1.parse::<u64>().ok() != Some(record.size)
        || parent.2.parse::<u128>().ok() != Some(record.modified_ms)
        || parent.3 != 1
        || !parent.4.is_empty()
        || parent.5.as_slice() != record.sha256.as_slice()
    {
        return Err(migration_collision(
            "legacy raw chunk parent identity collision",
        ));
    }

    let checkpoint: String = connection.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy raw chunk checkpoint is invalid"))?;
    let resume_record = checkpoint
        .get("source_record_ordinal")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let resume_offset = checkpoint
        .get("source_byte_offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let checkpoint_source = checkpoint
        .get("source_ordinal")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(source_ordinal as u64);
    if checkpoint_source != source_ordinal as u64
        || checkpoint
            .get("substep")
            .and_then(serde_json::Value::as_str)
            != Some("raw_chunks")
    {
        return Err(migration_collision(
            "legacy raw chunk checkpoint points to another substep",
        ));
    }
    let persisted: (i64, i64) = connection.query_row(
        "SELECT COUNT(*), COALESCE(SUM(length(raw)), 0)
         FROM legacy_raw_record_chunks
         WHERE migration_id = ?1 AND source_ordinal = ?2",
        params![migration_id, source_ordinal_i64],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if u64::try_from(persisted.0).ok() != Some(resume_record)
        || u64::try_from(persisted.1).ok() != Some(resume_offset)
    {
        return Err(migration_collision(
            "legacy raw chunks disagree with their checkpoint",
        ));
    }

    let path = app_data_root.join(&record.relative_path);
    let before = std::fs::metadata(&path)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    if before.len() != record.size
        || source_modified_ms(&before)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
            != record.modified_ms
    {
        return Err(migration_collision(
            "legacy source changed after migration inventory",
        ));
    }
    let mut source = std::fs::File::open(&path)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; RAW_CHUNK_LIMIT];
    let mut source_offset = 0_u64;
    let mut chunk_ordinal = 0_u64;
    let mut next_record = resume_record;
    let mut next_offset = resume_offset;
    loop {
        let mut filled = 0usize;
        while filled < buffer.len() {
            let read = source
                .read(&mut buffer[filled..])
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }
        let raw = &buffer[..filled];
        hasher.update(raw);
        let raw_sha256: [u8; 32] = Sha256::digest(raw).into();
        if chunk_ordinal < resume_record {
            let saved: (String, Vec<u8>, Vec<u8>) = connection.query_row(
                "SELECT source_offset, raw, raw_sha256
                 FROM legacy_raw_record_chunks
                 WHERE migration_id = ?1 AND source_ordinal = ?2 AND chunk_ordinal = ?3",
                params![migration_id, source_ordinal_i64, chunk_ordinal as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            if saved.0.parse::<u64>().ok() != Some(source_offset)
                || saved.1 != raw
                || saved.2.as_slice() != raw_sha256.as_slice()
            {
                return Err(migration_collision("legacy raw chunk identity collision"));
            }
        } else {
            if chunk_ordinal != next_record || source_offset != next_offset {
                return Err(migration_collision(
                    "legacy raw chunk resume position is not contiguous",
                ));
            }
            let committed_offset = source_offset
                .checked_add(filled as u64)
                .ok_or_else(|| migration_collision("legacy source offset overflow"))?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO legacy_raw_record_chunks
                    (migration_id, source_ordinal, chunk_ordinal, source_offset, raw, raw_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    migration_id,
                    source_ordinal_i64,
                    chunk_ordinal as i64,
                    source_offset.to_string(),
                    raw,
                    raw_sha256.as_slice(),
                ],
            )?;
            let mut checkpoint = chunk_checkpoint_value(&transaction, migration_id)?;
            let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                migration_collision("legacy raw chunk checkpoint is not an object")
            })?;
            checkpoint.insert(
                "source_record_ordinal".to_string(),
                serde_json::Value::from(chunk_ordinal.saturating_add(1)),
            );
            checkpoint.insert(
                "source_byte_offset".to_string(),
                serde_json::Value::from(committed_offset),
            );
            transaction.execute(
                "UPDATE local_store_migrations
                 SET checkpoint = ?2, revision = revision + 1, commit_id = ?3
                 WHERE migration_id = ?1",
                params![
                    migration_id,
                    serde_json::Value::Object(checkpoint.clone()).to_string(),
                    commit_id,
                ],
            )?;
            transaction.commit()?;
            next_record = chunk_ordinal.saturating_add(1);
            next_offset = committed_offset;
            at_checkpoint(connection);
        }
        source_offset = source_offset
            .checked_add(filled as u64)
            .ok_or_else(|| migration_collision("legacy source offset overflow"))?;
        chunk_ordinal = chunk_ordinal
            .checked_add(1)
            .ok_or_else(|| migration_collision("legacy raw chunk ordinal overflow"))?;
    }
    let after = std::fs::metadata(&path)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let digest: [u8; 32] = hasher.finalize().into();
    if source_offset != record.size
        || after.len() != record.size
        || source_modified_ms(&after)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?
            != record.modified_ms
        || digest != record.sha256
    {
        return Err(migration_collision(
            "legacy source changed after migration inventory",
        ));
    }

    let transaction = connection.transaction()?;
    let mut checkpoint = chunk_checkpoint_value(&transaction, migration_id)?;
    let checkpoint = checkpoint
        .as_object_mut()
        .ok_or_else(|| migration_collision("legacy raw chunk checkpoint is not an object"))?;
    checkpoint.insert(
        "substep".to_string(),
        serde_json::Value::String("source_complete".to_string()),
    );
    checkpoint.insert(
        "next_source_ordinal".to_string(),
        serde_json::Value::from(source_ordinal.saturating_add(1)),
    );
    checkpoint.insert(
        "imported_raw_record_count".to_string(),
        serde_json::Value::from(imported_raw_record_count.saturating_add(1)),
    );
    transaction.execute(
        "UPDATE local_store_migrations
         SET checkpoint = ?2, revision = revision + 1, commit_id = ?3
         WHERE migration_id = ?1",
        params![
            migration_id,
            serde_json::Value::Object(checkpoint.clone()).to_string(),
            commit_id,
        ],
    )?;
    transaction.commit()?;
    at_checkpoint(connection);
    Ok(())
}

fn migration_collision(reason: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(reason.to_string())))
}

fn legacy_message_ordinals(
    records: &[SourceRecord],
) -> Result<HashMap<String, i64>, rusqlite::Error> {
    let mut by_session = HashMap::<String, BTreeMap<u64, String>>::new();
    for record in records {
        let components = record.relative_path.split('/').collect::<Vec<_>>();
        let ["sessions", session_id, "messages", file_name] = components.as_slice() else {
            continue;
        };
        let raw_ordinal = file_name
            .strip_suffix(".json")
            .ok_or_else(|| migration_collision("legacy message ordinal is missing"))?;
        let ordinal = raw_ordinal
            .parse::<u64>()
            .ok()
            .filter(|ordinal| *ordinal > 0)
            .filter(|ordinal| raw_ordinal == ordinal.to_string())
            .ok_or_else(|| migration_collision("legacy message ordinal is invalid"))?;
        if ordinal > i64::MAX as u64 {
            return Err(migration_collision("legacy message ordinal exceeds SQLite"));
        }
        if by_session
            .entry((*session_id).to_string())
            .or_default()
            .insert(ordinal, record.relative_path.clone())
            .is_some()
        {
            return Err(migration_collision("legacy message ordinal is duplicated"));
        }
    }
    let mut ordinals = HashMap::new();
    for messages in by_session.into_values() {
        for (index, (ordinal, path)) in messages.into_iter().enumerate() {
            let expected = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| migration_collision("legacy message ordinal overflow"))?;
            if ordinal != expected {
                return Err(migration_collision(
                    "legacy message ordinal sequence has a gap",
                ));
            }
            ordinals.insert(path, ordinal as i64);
        }
    }
    Ok(ordinals)
}

fn insert_session_projection(
    transaction: &Transaction<'_>,
    session_id: &str,
    projection: &str,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT OR IGNORE INTO session_projection
            (session_id, projection, revision, commit_id)
         VALUES (?1, ?2, 0, ?3)",
        params![session_id, projection, commit_id],
    )?;
    let saved: Option<String> = transaction
        .query_row(
            "SELECT projection FROM session_projection WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    if saved.as_deref() != Some(projection) {
        return Err(migration_collision(
            "legacy session projection identity collision",
        ));
    }
    Ok(())
}

fn replace_session_projection(
    transaction: &Transaction<'_>,
    session_id: &str,
    expected_projection: &str,
    projection: &str,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    let changed = transaction.execute(
        "UPDATE session_projection SET projection = ?3, commit_id = ?4
         WHERE session_id = ?1 AND projection = ?2",
        params![session_id, expected_projection, projection, commit_id],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let saved: Option<String> = transaction
        .query_row(
            "SELECT projection FROM session_projection WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    if saved.as_deref() == Some(projection) {
        return Ok(());
    }
    Err(migration_collision(
        "legacy private context session projection changed",
    ))
}

fn insert_message_projection(
    transaction: &Transaction<'_>,
    session_id: &str,
    message_id: &str,
    message_ordinal: i64,
    projection: &str,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    if message_ordinal <= 0 {
        return Err(migration_collision("legacy message ordinal is invalid"));
    }
    transaction.execute(
        "INSERT OR IGNORE INTO message_projection
            (session_id, message_id, message_ordinal, projection, revision, commit_id)
         VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        params![
            session_id,
            message_id,
            message_ordinal,
            projection,
            commit_id
        ],
    )?;
    let saved: Option<(String, i64)> = transaction
        .query_row(
            "SELECT projection, message_ordinal FROM message_projection
             WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, message_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if saved
        .as_ref()
        .map(|(saved, ordinal)| (saved.as_str(), *ordinal))
        != Some((projection, message_ordinal))
    {
        return Err(migration_collision(
            "legacy message projection identity collision",
        ));
    }
    Ok(())
}

fn next_message_ordinal(
    transaction: &Transaction<'_>,
    session_id: &str,
) -> Result<i64, rusqlite::Error> {
    transaction.query_row(
        "SELECT COALESCE(MAX(message_ordinal), 0) + 1
         FROM message_projection WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )
}

#[allow(clippy::too_many_arguments)] // Every resumable participant stays explicit.
fn import_large_message_semantics<F>(
    connection: &mut Connection,
    app_data_root: &Path,
    records: &[SourceRecord],
    message_ordinals: &HashMap<String, i64>,
    migration_id: &str,
    commit_id: &str,
    initial_message_count: u64,
    at_checkpoint: &mut F,
) -> Result<u64, rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let checkpoint: String = connection.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy large-message checkpoint is invalid"))?;
    let mut after_source_ordinal = checkpoint
        .get("semantic_large_message_after_source_ordinal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let mut message_count = initial_message_count;

    for (source_ordinal, record) in records.iter().enumerate() {
        if record.size <= IMPORT_BYTE_LIMIT as u64
            || crate::adaptor::gateway::agent_session::session_storage::legacy_message_source_identity_v1(
                &record.relative_path,
            )
            .map_err(|reason| migration_collision(&reason))?
            .is_none()
            || after_source_ordinal.is_some_and(|after| source_ordinal <= after)
        {
            continue;
        }
        let decoded = decode_inventoried_large_message(app_data_root, record)
            .map_err(|reason| migration_collision(&reason))?;
        let crate::adaptor::gateway::agent_session::session_storage::LegacySessionProjectionV1::Message {
            session_id,
            message_id,
            projection,
        } = decoded
        else {
            return Err(migration_collision(
                "oversized legacy message decoded as another projection family",
            ));
        };
        let message_ordinal = message_ordinals
            .get(&record.relative_path)
            .copied()
            .ok_or_else(|| migration_collision("legacy message ordinal is missing"))?;
        let next_message_count = message_count
            .checked_add(1)
            .ok_or_else(|| migration_collision("legacy semantic message count overflow"))?;
        let transaction = connection.transaction()?;
        insert_message_projection(
            &transaction,
            &session_id,
            &message_id,
            message_ordinal,
            &projection,
            commit_id,
        )?;
        let current_checkpoint: String = transaction.query_row(
            "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
            params![migration_id],
            |row| row.get(0),
        )?;
        let mut checkpoint: serde_json::Value = serde_json::from_str(&current_checkpoint)
            .map_err(|_| migration_collision("legacy large-message checkpoint is invalid"))?;
        let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
            migration_collision("legacy large-message checkpoint is not an object")
        })?;
        checkpoint.insert(
            "substep".to_string(),
            serde_json::Value::String("semantic_large_message".to_string()),
        );
        checkpoint.insert(
            "semantic_source_kind".to_string(),
            serde_json::Value::String("agent_message".to_string()),
        );
        checkpoint.insert(
            "semantic_source_ordinal".to_string(),
            serde_json::Value::from(source_ordinal),
        );
        checkpoint.insert(
            "semantic_source_path".to_string(),
            serde_json::Value::String(record.relative_path.clone()),
        );
        checkpoint.insert(
            "semantic_large_message_after_source_ordinal".to_string(),
            serde_json::Value::from(source_ordinal),
        );
        checkpoint.insert(
            "semantic_next_record_ordinal".to_string(),
            serde_json::Value::from(1),
        );
        checkpoint.insert(
            "semantic_next_event_ordinal".to_string(),
            serde_json::Value::from(0),
        );
        checkpoint.insert(
            "semantic_next_chunk_index".to_string(),
            serde_json::Value::from(1),
        );
        checkpoint.insert(
            "semantic_source_byte_offset".to_string(),
            serde_json::Value::from(record.size),
        );
        checkpoint.insert(
            "source_byte_offset".to_string(),
            serde_json::Value::from(record.size),
        );
        checkpoint.insert(
            "semantic_chunk_record_count".to_string(),
            serde_json::Value::from(1),
        );
        checkpoint.insert(
            "semantic_chunk_event_count".to_string(),
            serde_json::Value::from(0),
        );
        checkpoint.insert(
            "semantic_chunk_decoded_bytes".to_string(),
            serde_json::Value::from(projection.len()),
        );
        checkpoint.insert(
            "semantic_message_count".to_string(),
            serde_json::Value::from(next_message_count),
        );
        transaction.execute(
            "UPDATE local_store_migrations
             SET phase = 'importing', checkpoint = ?2, revision = revision + 1,
                 commit_id = ?3 WHERE migration_id = ?1",
            params![
                migration_id,
                serde_json::Value::Object(checkpoint.clone()).to_string(),
                commit_id,
            ],
        )?;
        transaction.commit()?;
        message_count = next_message_count;
        after_source_ordinal = Some(source_ordinal);
        at_checkpoint(connection);
    }
    Ok(message_count)
}

fn verify_semantic_projections<F>(
    connection: &Connection,
    expected_sessions: u64,
    expected_messages: u64,
    expected_workflows: u64,
    expected_agent: AgentSemanticMigrationStats,
    mut at_checkpoint: F,
) -> Result<AgentProjectionFieldParity, rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let codec =
        crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1;
    let mut session_count = 0_u64;
    let mut titled_session_count = 0_u64;
    let mut pending_queue_count = 0_u64;
    let mut pending_permission_count = 0_u64;
    let mut workflow_instruction_count = 0_u64;
    let mut workflow_instruction_hasher = Sha256::new();
    workflow_instruction_hasher.update(b"migration-workflow-instructions/v1\0");
    let mut context_epoch_payload_count = 0_u64;
    let mut context_epoch_payload_hasher = Sha256::new();
    context_epoch_payload_hasher.update(b"migration-context-epoch-payloads/v1\0");
    let mut agent_read_path_count = 0_u64;
    let mut agent_read_path_hasher = Sha256::new();
    agent_read_path_hasher.update(b"migration-agent-read-paths/v1\0");
    let mut owner_relation_count = 0_u64;
    let mut owner_relation_hasher = Sha256::new();
    owner_relation_hasher.update(b"migration-owner-relations/v1\0");
    let mut after_session_id: Option<String> = None;
    loop {
        let rows = {
            let mut statement = connection.prepare(
                "SELECT session_id, projection FROM session_projection
                 WHERE session_id NOT LIKE 'workflow:%'
                   AND (?1 IS NULL OR session_id > ?1)
                 ORDER BY session_id ASC LIMIT 256",
            )?;
            let rows = statement
                .query_map(params![after_session_id.as_deref()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (session_id, projection) in &rows {
            let payload = decode_session_projection_record_v1(projection, session_id)
                .map_err(|error| migration_collision(&error))?;
            let decoded = codec
                .decode(&payload)
                .map_err(|error| migration_collision(&error))?;
            semantic_hash_field(&mut workflow_instruction_hasher, session_id.as_bytes());
            workflow_instruction_hasher.update(
                u64::try_from(decoded.meta.workflow_instructions.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            for instruction in &decoded.meta.workflow_instructions {
                semantic_hash_field(&mut workflow_instruction_hasher, instruction.as_bytes());
                workflow_instruction_count =
                    workflow_instruction_count.checked_add(1).ok_or_else(|| {
                        migration_collision("semantic workflow-instruction count overflow")
                    })?;
            }

            let context_payloads = decoded
                .meta
                .context_epoch
                .as_ref()
                .map(|epoch| epoch.payload_cache_entries())
                .unwrap_or_default();
            semantic_hash_field(&mut context_epoch_payload_hasher, session_id.as_bytes());
            context_epoch_payload_hasher.update(
                u64::try_from(context_payloads.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            for payload in &context_payloads {
                semantic_hash_field(&mut context_epoch_payload_hasher, payload.kind.as_bytes());
                semantic_hash_optional_text(
                    &mut context_epoch_payload_hasher,
                    payload.fingerprint.as_deref(),
                );
                semantic_hash_field(
                    &mut context_epoch_payload_hasher,
                    payload.payload.as_bytes(),
                );
                context_epoch_payload_count =
                    context_epoch_payload_count.checked_add(1).ok_or_else(|| {
                        migration_collision("semantic context-epoch payload count overflow")
                    })?;
            }

            semantic_hash_field(&mut agent_read_path_hasher, session_id.as_bytes());
            match decoded.meta.agent_read_paths.as_ref() {
                Some(paths) => {
                    agent_read_path_hasher.update([1]);
                    agent_read_path_hasher
                        .update(u64::try_from(paths.len()).unwrap_or(u64::MAX).to_be_bytes());
                    for path in paths {
                        let path = path.to_str().ok_or_else(|| {
                            migration_collision("semantic agent read path is not UTF-8")
                        })?;
                        semantic_hash_field(&mut agent_read_path_hasher, path.as_bytes());
                        agent_read_path_count =
                            agent_read_path_count.checked_add(1).ok_or_else(|| {
                                migration_collision("semantic agent-read-path count overflow")
                            })?;
                    }
                }
                None => agent_read_path_hasher.update([0]),
            }

            let owner = decoded.meta.workflow_node_context.as_ref();
            if decoded.meta.workflow_node_session || owner.is_some() {
                owner_relation_count = owner_relation_count
                    .checked_add(1)
                    .ok_or_else(|| migration_collision("semantic owner-relation count overflow"))?;
                semantic_hash_field(&mut owner_relation_hasher, session_id.as_bytes());
                owner_relation_hasher.update([u8::from(decoded.meta.workflow_node_session)]);
                match owner {
                    Some(owner) => {
                        owner_relation_hasher.update([1]);
                        semantic_hash_field(
                            &mut owner_relation_hasher,
                            owner.execution_id.as_bytes(),
                        );
                        semantic_hash_field(
                            &mut owner_relation_hasher,
                            owner.node_execution_id.as_bytes(),
                        );
                        semantic_hash_field(
                            &mut owner_relation_hasher,
                            owner.workflow_name.as_bytes(),
                        );
                        semantic_hash_field(&mut owner_relation_hasher, owner.node_name.as_bytes());
                        owner_relation_hasher.update(owner.attempt.to_be_bytes());
                        semantic_hash_optional_text(
                            &mut owner_relation_hasher,
                            owner.parent_node_name.as_deref(),
                        );
                        semantic_hash_optional_u32(
                            &mut owner_relation_hasher,
                            owner.parent_attempt,
                        );
                        owner_relation_hasher.update(owner.order.to_be_bytes());
                        semantic_hash_optional_u64(
                            &mut owner_relation_hasher,
                            owner.startup_timeout_secs,
                        );
                        semantic_hash_optional_u32(
                            &mut owner_relation_hasher,
                            owner.startup_max_retries,
                        );
                        semantic_hash_optional_u64(
                            &mut owner_relation_hasher,
                            owner.stale_timeout_secs,
                        );
                    }
                    None => owner_relation_hasher.update([0]),
                }
            }
            titled_session_count = titled_session_count
                .checked_add(u64::from(decoded.title.is_some()))
                .ok_or_else(|| migration_collision("semantic title count overflow"))?;
            pending_queue_count = pending_queue_count
                .checked_add(
                    u64::try_from(decoded.pending_send_queue.len())
                        .map_err(|_| migration_collision("semantic queue count overflow"))?,
                )
                .ok_or_else(|| migration_collision("semantic queue count overflow"))?;
            if crate::usecase::agent_session::event_log::latest_unresolved_permission_request(
                &decoded.reducer_events,
            )
            .is_some()
            {
                pending_permission_count = pending_permission_count
                    .checked_add(1)
                    .ok_or_else(|| migration_collision("semantic permission count overflow"))?;
            }
            for event in &decoded.reducer_events {
                let turn_id = match event {
                    crate::usecase::agent_session::event_log::AgentSessionEvent::TurnCompleted {
                        turn_id,
                        ..
                    }
                    | crate::usecase::agent_session::event_log::AgentSessionEvent::TurnInterrupted {
                        turn_id,
                        ..
                    } => turn_id,
                    _ => continue,
                };
                let exists: i64 = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM terminal_records
                     WHERE session_id = ?1 AND turn_id = ?2)",
                    params![session_id, turn_id.to_string()],
                    |row| row.get(0),
                )?;
                if exists != 1 {
                    return Err(migration_collision(
                        "semantic terminal projection is missing its direct record",
                    ));
                }
            }
            session_count = session_count.saturating_add(1);
        }
        after_session_id = rows.last().map(|(session_id, _)| session_id.clone());
        at_checkpoint(connection);
    }

    let mut message_count = 0_u64;
    let mut after_message: Option<(String, String)> = None;
    loop {
        let rows = {
            let mut statement = connection.prepare(
                "SELECT session_id, message_id, projection FROM message_projection
                 WHERE session_id NOT LIKE 'blob:%'
                   AND (?1 IS NULL OR session_id > ?1
                        OR (session_id = ?1 AND message_id > ?2))
                 ORDER BY session_id ASC, message_id ASC LIMIT 256",
            )?;
            let rows = statement
                .query_map(
                    params![
                        after_message
                            .as_ref()
                            .map(|(session_id, _)| session_id.as_str()),
                        after_message
                            .as_ref()
                            .map(|(_, message_id)| message_id.as_str()),
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (session_id, message_id, projection) in &rows {
            decode_message_projection_record_v1(projection, session_id, message_id)
                .map_err(|error| migration_collision(&error))?;
            message_count = message_count.saturating_add(1);
        }
        after_message = rows
            .last()
            .map(|(session_id, message_id, _)| (session_id.clone(), message_id.clone()));
        at_checkpoint(connection);
    }

    let mut workflow_count = 0_u64;
    let mut after_workflow_id: Option<String> = None;
    loop {
        let rows = {
            let mut statement = connection.prepare(
                "SELECT session_id, projection FROM session_projection
                 WHERE session_id LIKE 'workflow:%'
                   AND (?1 IS NULL OR session_id > ?1)
                 ORDER BY session_id ASC LIMIT 256",
            )?;
            let rows = statement
                .query_map(params![after_workflow_id.as_deref()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (_, projection) in &rows {
            crate::adaptor::gateway::workflow::execution_store::workflow_projection_is_non_terminal(
                projection,
            )
            .map_err(|error| migration_collision(&error))?;
            workflow_count = workflow_count.saturating_add(1);
        }
        after_workflow_id = rows.last().map(|(session_id, _)| session_id.clone());
        at_checkpoint(connection);
    }
    if (session_count, message_count, workflow_count)
        != (expected_sessions, expected_messages, expected_workflows)
    {
        return Err(migration_collision("semantic projection parity mismatch"));
    }
    let terminal_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM terminal_records WHERE commit_id LIKE 'migration-%'",
        [],
        |row| row.get(0),
    )?;
    let stop_resolution_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM stop_resolutions WHERE commit_id LIKE 'migration-%'",
        [],
        |row| row.get(0),
    )?;
    let pending_obligation_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM obligations
         WHERE obligation_id LIKE 'legacy-%' AND pending = 1",
        [],
        |row| row.get(0),
    )?;
    let indexed_pending_obligation_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pending_obligations
         WHERE obligation_id LIKE 'legacy-%'",
        [],
        |row| row.get(0),
    )?;
    let actual_agent = AgentSemanticMigrationStats {
        terminal_count: u64::try_from(terminal_count)
            .map_err(|_| migration_collision("semantic terminal count is invalid"))?,
        stop_resolution_count: u64::try_from(stop_resolution_count)
            .map_err(|_| migration_collision("semantic Stop resolution count is invalid"))?,
        pending_obligation_count: u64::try_from(pending_obligation_count)
            .map_err(|_| migration_collision("semantic obligation count is invalid"))?,
        pending_queue_count,
        pending_permission_count,
        titled_session_count,
    };
    if actual_agent != expected_agent
        || pending_obligation_count != indexed_pending_obligation_count
    {
        return Err(migration_collision(
            "agent semantic terminal/permission/queue parity mismatch",
        ));
    }
    Ok(AgentProjectionFieldParity {
        workflow_instruction_count,
        workflow_instruction_sha256: workflow_instruction_hasher.finalize().into(),
        context_epoch_payload_count,
        context_epoch_payload_sha256: context_epoch_payload_hasher.finalize().into(),
        agent_read_path_count,
        agent_read_path_sha256: agent_read_path_hasher.finalize().into(),
        owner_relation_count,
        owner_relation_sha256: owner_relation_hasher.finalize().into(),
    })
}

fn insert_workflow_execution_obligation(
    transaction: &Transaction<'_>,
    execution_id: &str,
    projection: &str,
    pending: bool,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    let obligation_id = format!("workflow-execution-{execution_id}");
    transaction.execute(
        "INSERT OR IGNORE INTO obligations
            (obligation_id, record, pending, revision, commit_id)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![obligation_id, projection, i64::from(pending), commit_id],
    )?;
    let saved: Option<(String, i64)> = transaction
        .query_row(
            "SELECT record, revision FROM obligations WHERE obligation_id = ?1",
            params![obligation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if saved
        .as_ref()
        .map(|(record, revision)| (record.as_str(), *revision))
        != Some((projection, 0))
    {
        return Err(migration_collision(
            "legacy workflow recovery identity collision",
        ));
    }
    transaction.execute(
        "DELETE FROM pending_obligations WHERE obligation_id = ?1",
        params![obligation_id],
    )?;
    if pending {
        transaction.execute(
            "INSERT INTO pending_obligations
                (ordered_key, obligation_id, owner, partition,
                 shutdown_plan_id, shutdown_epoch, commit_id)
             VALUES (?1, ?2, 'workflow-runtime', 'unowned_runtime', NULL, NULL, ?3)",
            params![
                format!("workflow_execution:{execution_id}"),
                obligation_id,
                commit_id
            ],
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AgentSemanticMigrationStats {
    terminal_count: u64,
    stop_resolution_count: u64,
    pending_obligation_count: u64,
    pending_queue_count: u64,
    pending_permission_count: u64,
    titled_session_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentProjectionFieldParity {
    workflow_instruction_count: u64,
    workflow_instruction_sha256: [u8; 32],
    context_epoch_payload_count: u64,
    context_epoch_payload_sha256: [u8; 32],
    agent_read_path_count: u64,
    agent_read_path_sha256: [u8; 32],
    owner_relation_count: u64,
    owner_relation_sha256: [u8; 32],
}

fn semantic_hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn semantic_hash_optional_text(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            semantic_hash_field(hasher, value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn semantic_hash_optional_u32(hasher: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn semantic_hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        None => hasher.update([0]),
    }
}

fn parity_hash32(
    parity: &serde_json::Value,
    key: &str,
    missing_reason: &str,
) -> Result<[u8; 32], rusqlite::Error> {
    let raw = parity
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| migration_collision(missing_reason))?;
    let decoded = hex::decode(raw).map_err(|_| migration_collision(missing_reason))?;
    decoded
        .try_into()
        .map_err(|_| migration_collision(missing_reason))
}

fn semantic_projection_field_parity(
    parity: &serde_json::Value,
) -> Result<AgentProjectionFieldParity, rusqlite::Error> {
    Ok(AgentProjectionFieldParity {
        workflow_instruction_count: parity_u64(
            parity,
            "semantic_workflow_instruction_count",
            "migration workflow-instruction parity count is missing",
        )?,
        workflow_instruction_sha256: parity_hash32(
            parity,
            "semantic_workflow_instruction_sha256",
            "migration workflow-instruction parity digest is missing",
        )?,
        context_epoch_payload_count: parity_u64(
            parity,
            "semantic_context_epoch_payload_count",
            "migration context-epoch payload parity count is missing",
        )?,
        context_epoch_payload_sha256: parity_hash32(
            parity,
            "semantic_context_epoch_payload_sha256",
            "migration context-epoch payload parity digest is missing",
        )?,
        agent_read_path_count: parity_u64(
            parity,
            "semantic_agent_read_path_count",
            "migration agent-read-path parity count is missing",
        )?,
        agent_read_path_sha256: parity_hash32(
            parity,
            "semantic_agent_read_path_sha256",
            "migration agent-read-path parity digest is missing",
        )?,
        owner_relation_count: parity_u64(
            parity,
            "semantic_owner_relation_count",
            "migration owner-relation parity count is missing",
        )?,
        owner_relation_sha256: parity_hash32(
            parity,
            "semantic_owner_relation_sha256",
            "migration owner-relation parity digest is missing",
        )?,
    })
}

fn parity_u64(
    parity: &serde_json::Value,
    key: &str,
    missing_reason: &str,
) -> Result<u64, rusqlite::Error> {
    parity
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| migration_collision(missing_reason))
}

fn semantic_parity_counts(
    parity: &serde_json::Value,
) -> Result<(u64, u64, u64, u64, AgentSemanticMigrationStats), rusqlite::Error> {
    Ok((
        parity_u64(
            parity,
            "semantic_session_count",
            "migration session parity count is missing",
        )?,
        parity_u64(
            parity,
            "semantic_message_count",
            "migration message parity count is missing",
        )?,
        parity_u64(
            parity,
            "semantic_workflow_count",
            "migration workflow parity count is missing",
        )?,
        parity_u64(
            parity,
            "semantic_event_count",
            "migration event parity count is missing",
        )?,
        AgentSemanticMigrationStats {
            terminal_count: parity_u64(
                parity,
                "semantic_terminal_count",
                "migration terminal parity count is missing",
            )?,
            stop_resolution_count: parity_u64(
                parity,
                "semantic_stop_resolution_count",
                "migration Stop resolution parity count is missing",
            )?,
            pending_obligation_count: parity_u64(
                parity,
                "semantic_agent_pending_obligation_count",
                "migration obligation parity count is missing",
            )?,
            pending_queue_count: parity_u64(
                parity,
                "semantic_pending_queue_count",
                "migration queue parity count is missing",
            )?,
            pending_permission_count: parity_u64(
                parity,
                "semantic_pending_permission_count",
                "migration permission parity count is missing",
            )?,
            titled_session_count: parity_u64(
                parity,
                "semantic_titled_session_count",
                "migration title parity count is missing",
            )?,
        },
    ))
}

fn stored_semantic_event_count(connection: &Connection) -> Result<u64, rusqlite::Error> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM events AS event
         JOIN logical_commits AS logical_commit
           ON logical_commit.commit_id = event.commit_id
         WHERE logical_commit.operation_kind = 'migration'",
        [],
        |row| row.get(0),
    )?;
    u64::try_from(count).map_err(|_| migration_collision("migration event count is invalid"))
}

fn hash_proof_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

/// Seal the fact that full mutable parity succeeded at the authority boundary.
/// The proof deliberately binds only immutable migration identity and the
/// persisted attestation. Current projections and pending indexes are allowed
/// to evolve normally after cutover and therefore cannot be reopen inputs.
#[allow(clippy::too_many_arguments)] // Every argument is an independently sealed proof field.
fn activation_proof_digest(
    migration_id: &str,
    commit_id: &str,
    generation_id: &str,
    migration_revision: i64,
    mutation_count: i64,
    source_inventory_hash: &[u8; 32],
    checkpoint: &str,
    parity: &str,
) -> [u8; 32] {
    let mut proof = Sha256::new();
    proof.update(b"legacy-migration-activation-proof/v1\0");
    hash_proof_field(&mut proof, migration_id.as_bytes());
    hash_proof_field(&mut proof, commit_id.as_bytes());
    hash_proof_field(&mut proof, generation_id.as_bytes());
    proof.update(migration_revision.to_be_bytes());
    proof.update(mutation_count.to_be_bytes());
    proof.update(source_inventory_hash);
    hash_proof_field(&mut proof, checkpoint.as_bytes());
    hash_proof_field(&mut proof, parity.as_bytes());
    proof.finalize().into()
}

fn checked_semantic_count(value: usize, label: &str) -> Result<u64, rusqlite::Error> {
    u64::try_from(value).map_err(|_| migration_collision(label))
}

fn insert_legacy_terminal(
    transaction: &Transaction<'_>,
    terminal: &crate::adaptor::gateway::agent_session::session_storage::LegacyTurnTerminalV1,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    validate_legacy_state_record_bound(
        &terminal.result,
        "legacy terminal exceeds its bounded record",
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO terminal_records
            (session_id, turn_id, terminal_identity, result, participant_digest, commit_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            terminal.session_id,
            terminal.turn_id,
            terminal.terminal_identity,
            terminal.result,
            terminal.participant_digest.as_slice(),
            commit_id,
        ],
    )?;
    let saved: Option<(String, String, Vec<u8>)> = transaction
        .query_row(
            "SELECT terminal_identity, result, participant_digest
             FROM terminal_records WHERE session_id = ?1 AND turn_id = ?2",
            params![terminal.session_id, terminal.turn_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if saved
        .as_ref()
        .map(|(identity, result, digest)| (identity.as_str(), result.as_str(), digest.as_slice()))
        != Some((
            terminal.terminal_identity.as_str(),
            terminal.result.as_str(),
            terminal.participant_digest.as_slice(),
        ))
    {
        return Err(migration_collision(
            "legacy terminal projection identity collision",
        ));
    }
    Ok(())
}

fn insert_legacy_stop_resolution(
    transaction: &Transaction<'_>,
    resolution: &crate::adaptor::gateway::agent_session::session_storage::LegacyStopResolutionV1,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    validate_legacy_state_record_bound(
        &resolution.detail,
        "legacy Stop resolution exceeds its bounded record",
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO stop_resolutions
            (stop_operation_id, resolution, detail, commit_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            resolution.stop_operation_id,
            resolution.resolution,
            resolution.detail,
            commit_id,
        ],
    )?;
    let saved: Option<(String, String)> = transaction
        .query_row(
            "SELECT resolution, detail FROM stop_resolutions
             WHERE stop_operation_id = ?1",
            params![resolution.stop_operation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if saved
        .as_ref()
        .map(|(kind, detail)| (kind.as_str(), detail.as_str()))
        != Some((resolution.resolution, resolution.detail.as_str()))
    {
        return Err(migration_collision(
            "legacy Stop resolution identity collision",
        ));
    }
    Ok(())
}

fn insert_legacy_agent_obligation(
    transaction: &Transaction<'_>,
    obligation: &crate::adaptor::gateway::agent_session::session_storage::LegacyAgentObligationV1,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    validate_legacy_state_record_bound(
        &obligation.record,
        "legacy agent obligation exceeds its bounded record",
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO obligations
            (obligation_id, record, pending, revision, commit_id)
         VALUES (?1, ?2, 1, 0, ?3)",
        params![obligation.obligation_id, obligation.record, commit_id],
    )?;
    let saved: Option<(String, i64, i64)> = transaction
        .query_row(
            "SELECT record, pending, revision FROM obligations WHERE obligation_id = ?1",
            params![obligation.obligation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if saved
        .as_ref()
        .map(|(record, pending, revision)| (record.as_str(), *pending, *revision))
        != Some((obligation.record.as_str(), 1, 0))
    {
        return Err(migration_collision(
            "legacy agent obligation identity collision",
        ));
    }
    transaction.execute(
        "INSERT OR IGNORE INTO pending_obligations
            (ordered_key, obligation_id, owner, partition,
             shutdown_plan_id, shutdown_epoch, commit_id)
         VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5)",
        params![
            obligation.ordered_key,
            obligation.obligation_id,
            obligation.owner,
            obligation.partition,
            commit_id,
        ],
    )?;
    let pending: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT ordered_key, owner, partition FROM pending_obligations
             WHERE obligation_id = ?1",
            params![obligation.obligation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if pending.as_ref().map(|(ordered_key, owner, partition)| {
        (ordered_key.as_str(), owner.as_str(), partition.as_str())
    }) != Some((
        obligation.ordered_key.as_str(),
        obligation.owner.as_str(),
        obligation.partition,
    )) {
        return Err(migration_collision(
            "legacy agent pending index identity collision",
        ));
    }
    Ok(())
}

fn insert_legacy_semantic_participant(
    transaction: &Transaction<'_>,
    participant: &crate::adaptor::gateway::agent_session::session_storage::LegacySessionSemanticParticipantV1,
    commit_id: &str,
) -> Result<(), rusqlite::Error> {
    use crate::adaptor::gateway::agent_session::session_storage::LegacySessionSemanticParticipantV1;

    match participant {
        LegacySessionSemanticParticipantV1::Terminal(terminal) => {
            insert_legacy_terminal(transaction, terminal, commit_id)
        }
        LegacySessionSemanticParticipantV1::StopResolution(resolution) => {
            insert_legacy_stop_resolution(transaction, resolution, commit_id)
        }
        LegacySessionSemanticParticipantV1::PendingObligation(obligation) => {
            insert_legacy_agent_obligation(transaction, obligation, commit_id)
        }
    }
}

fn checkpoint_u64(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)] // Keeps every migration input explicit at the one-shot boundary.
fn materialize_legacy_agent_semantics<F>(
    connection: &mut Connection,
    app_data_root: &Path,
    records: &[SourceRecord],
    event_sources: &[(usize, String, (u8, u64))],
    titles: &std::collections::HashMap<String, String>,
    migration_id: &str,
    commit_id: &str,
    at_checkpoint: &mut F,
) -> Result<AgentSemanticMigrationStats, rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let checkpoint: String = connection.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
    let mut after_session_id = checkpoint
        .get("semantic_session_after")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let mut stats = AgentSemanticMigrationStats {
        terminal_count: checkpoint_u64(&checkpoint, "semantic_terminal_count"),
        stop_resolution_count: checkpoint_u64(&checkpoint, "semantic_stop_resolution_count"),
        pending_obligation_count: checkpoint_u64(
            &checkpoint,
            "semantic_agent_pending_obligation_count",
        ),
        pending_queue_count: checkpoint_u64(&checkpoint, "semantic_pending_queue_count"),
        pending_permission_count: checkpoint_u64(&checkpoint, "semantic_pending_permission_count"),
        titled_session_count: checkpoint_u64(&checkpoint, "semantic_titled_session_count"),
    };
    let materialization_resume = (checkpoint
        .get("substep")
        .and_then(serde_json::Value::as_str)
        == Some("semantic_session_events"))
    .then(|| {
        Some((
            checkpoint.get("semantic_session_id")?.as_str()?.to_string(),
            usize::try_from(checkpoint.get("semantic_source_ordinal")?.as_u64()?).ok()?,
            checkpoint.get("semantic_next_record_ordinal")?.as_u64()?,
        ))
    })
    .flatten();

    let mut source_indices = std::collections::HashMap::<String, Vec<usize>>::new();
    for (record_index, session_id, _) in event_sources {
        source_indices
            .entry(session_id.clone())
            .or_default()
            .push(*record_index);
    }
    for session_id in source_indices.keys() {
        let exists: i64 = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_projection WHERE session_id = ?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Err(migration_collision(
                "known legacy agent event stream has no session projection",
            ));
        }
    }

    loop {
        let rows = {
            let mut statement = connection.prepare(
                "SELECT session_id, projection FROM session_projection
                 WHERE session_id NOT LIKE 'workflow:%'
                   AND (?1 IS NULL OR session_id > ?1)
                 ORDER BY session_id ASC LIMIT 256",
            )?;
            let rows = statement
                .query_map(params![after_session_id.as_deref()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        if rows.is_empty() {
            break;
        }
        for (session_id, base_projection) in rows {
            let mut accumulator = crate::adaptor::gateway::agent_session::session_storage::LegacySessionSemanticAccumulatorV1::new(
                &session_id,
                &base_projection,
                titles.get(&session_id).map(String::as_str),
            )
            .map_err(|reason| migration_collision(&reason))?;
            if let Some(indices) = source_indices.get(&session_id) {
                for record_index in indices {
                    let record = &records[*record_index];
                    let suppress_through_record_ordinal = match &materialization_resume {
                        Some((resume_session, resume_source, _))
                            if resume_session == &session_id && *record_index < *resume_source =>
                        {
                            u64::MAX
                        }
                        Some((resume_session, resume_source, resume_record))
                            if resume_session == &session_id && *record_index == *resume_source =>
                        {
                            *resume_record
                        }
                        _ => 0,
                    };
                    let mut source_event_ordinal = 0_u64;
                    stream_inventoried_json_array(
                        app_data_root,
                        record,
                        |record_ordinal, raw| {
                            let decoded = crate::adaptor::gateway::agent_session::session_storage::decode_legacy_agent_event_record_v1(
                                raw,
                                &record.relative_path,
                                source_event_ordinal,
                            )?;
                            let event_count = decoded.len();
                            let mut participants = Vec::new();
                            for event in decoded {
                                if let Some(participant) = accumulator.push(event)? {
                                    participants.push(participant);
                                }
                            }
                            source_event_ordinal = source_event_ordinal
                                .checked_add(u64::try_from(event_count).map_err(|_| {
                                    "legacy semantic event ordinal overflow".to_string()
                                })?)
                                .ok_or_else(|| {
                                    "legacy semantic event ordinal overflow".to_string()
                                })?;
                            if record_ordinal.saturating_add(1)
                                <= suppress_through_record_ordinal
                            {
                                return Ok(());
                            }
                            let transaction = connection
                                .transaction()
                                .map_err(|error| error.to_string())?;
                            for participant in &participants {
                                insert_legacy_semantic_participant(
                                    &transaction,
                                    participant,
                                    commit_id,
                                )
                                .map_err(|error| error.to_string())?;
                            }
                            let current_checkpoint: String = transaction
                                .query_row(
                                    "SELECT checkpoint FROM local_store_migrations
                                     WHERE migration_id = ?1",
                                    params![migration_id],
                                    |row| row.get(0),
                                )
                                .map_err(|error| error.to_string())?;
                            let mut checkpoint: serde_json::Value =
                                serde_json::from_str(&current_checkpoint).map_err(|_| {
                                    "legacy semantic checkpoint is invalid".to_string()
                                })?;
                            let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                                "legacy semantic checkpoint is not an object".to_string()
                            })?;
                            checkpoint.insert(
                                "substep".to_string(),
                                serde_json::Value::String(
                                    "semantic_session_events".to_string(),
                                ),
                            );
                            checkpoint.insert(
                                "semantic_session_id".to_string(),
                                serde_json::Value::String(session_id.clone()),
                            );
                            checkpoint.insert(
                                "semantic_source_ordinal".to_string(),
                                serde_json::Value::from(*record_index),
                            );
                            checkpoint.insert(
                                "semantic_source_path".to_string(),
                                serde_json::Value::String(record.relative_path.clone()),
                            );
                            checkpoint.insert(
                                "semantic_next_record_ordinal".to_string(),
                                serde_json::Value::from(record_ordinal.saturating_add(1)),
                            );
                            checkpoint.insert(
                                "semantic_next_event_ordinal".to_string(),
                                serde_json::Value::from(source_event_ordinal),
                            );
                            checkpoint.insert(
                                "semantic_materialization_record_event_count".to_string(),
                                serde_json::Value::from(event_count),
                            );
                            checkpoint.insert(
                                "semantic_materialization_participant_count".to_string(),
                                serde_json::Value::from(participants.len()),
                            );
                            transaction
                                .execute(
                                    "UPDATE local_store_migrations
                                     SET phase = 'importing', checkpoint = ?2,
                                         revision = revision + 1, commit_id = ?3
                                     WHERE migration_id = ?1",
                                    params![
                                        migration_id,
                                        serde_json::Value::Object(checkpoint.clone()).to_string(),
                                        commit_id,
                                    ],
                                )
                                .map_err(|error| error.to_string())?;
                            transaction.commit().map_err(|error| error.to_string())?;
                            at_checkpoint(connection);
                            Ok(())
                        },
                    )
                    .map_err(|reason| migration_collision(&reason))?;
                }
            }
            let materialized = accumulator
                .finish()
                .map_err(|reason| migration_collision(&reason))?;
            let transaction = connection.transaction()?;
            let changed = transaction.execute(
                "UPDATE session_projection SET projection = ?2, commit_id = ?3
                 WHERE session_id = ?1",
                params![session_id, materialized.projection, commit_id],
            )?;
            if changed != 1 {
                return Err(migration_collision(
                    "legacy semantic session projection disappeared",
                ));
            }
            for obligation in &materialized.pending_obligations {
                insert_legacy_agent_obligation(&transaction, obligation, commit_id)?;
            }
            let terminal_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM terminal_records
                 WHERE session_id = ?1 AND commit_id = ?2",
                params![session_id, commit_id],
                |row| row.get(0),
            )?;
            let stop_resolution_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM stop_resolutions
                 WHERE commit_id = ?1
                   AND json_extract(detail, '$.session_id') = ?2",
                params![commit_id, session_id],
                |row| row.get(0),
            )?;
            let pending_obligation_count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM obligations AS obligation
                 JOIN pending_obligations AS pending
                   ON pending.obligation_id = obligation.obligation_id
                 WHERE obligation.commit_id = ?1 AND pending.owner = ?2",
                params![commit_id, session_id],
                |row| row.get(0),
            )?;
            let session_stats = AgentSemanticMigrationStats {
                terminal_count: u64::try_from(terminal_count)
                    .map_err(|_| migration_collision("legacy terminal count overflow"))?,
                stop_resolution_count: u64::try_from(stop_resolution_count)
                    .map_err(|_| migration_collision("legacy Stop resolution count overflow"))?,
                pending_obligation_count: u64::try_from(pending_obligation_count)
                    .map_err(|_| migration_collision("legacy pending obligation count overflow"))?,
                pending_queue_count: checked_semantic_count(
                    materialized.pending_queue_count,
                    "legacy pending queue count overflow",
                )?,
                pending_permission_count: checked_semantic_count(
                    materialized.pending_permission_count,
                    "legacy pending permission count overflow",
                )?,
                titled_session_count: u64::from(titles.contains_key(&session_id)),
            };
            let next_stats = AgentSemanticMigrationStats {
                terminal_count: stats
                    .terminal_count
                    .checked_add(session_stats.terminal_count)
                    .ok_or_else(|| migration_collision("legacy terminal count overflow"))?,
                stop_resolution_count: stats
                    .stop_resolution_count
                    .checked_add(session_stats.stop_resolution_count)
                    .ok_or_else(|| migration_collision("legacy Stop resolution count overflow"))?,
                pending_obligation_count: stats
                    .pending_obligation_count
                    .checked_add(session_stats.pending_obligation_count)
                    .ok_or_else(|| {
                        migration_collision("legacy pending obligation count overflow")
                    })?,
                pending_queue_count: stats
                    .pending_queue_count
                    .checked_add(session_stats.pending_queue_count)
                    .ok_or_else(|| migration_collision("legacy pending queue count overflow"))?,
                pending_permission_count: stats
                    .pending_permission_count
                    .checked_add(session_stats.pending_permission_count)
                    .ok_or_else(|| {
                        migration_collision("legacy pending permission count overflow")
                    })?,
                titled_session_count: stats
                    .titled_session_count
                    .checked_add(session_stats.titled_session_count)
                    .ok_or_else(|| migration_collision("legacy title count overflow"))?,
            };
            let current_checkpoint: String = transaction.query_row(
                "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
                params![migration_id],
                |row| row.get(0),
            )?;
            let mut checkpoint: serde_json::Value = serde_json::from_str(&current_checkpoint)
                .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
            let checkpoint = checkpoint.as_object_mut().ok_or_else(|| {
                migration_collision("legacy semantic checkpoint is not an object")
            })?;
            checkpoint.insert(
                "substep".to_string(),
                serde_json::Value::String("semantic_session_projection".to_string()),
            );
            checkpoint.insert(
                "semantic_session_after".to_string(),
                serde_json::Value::String(session_id.clone()),
            );
            checkpoint.insert(
                "semantic_terminal_count".to_string(),
                serde_json::Value::from(next_stats.terminal_count),
            );
            checkpoint.insert(
                "semantic_stop_resolution_count".to_string(),
                serde_json::Value::from(next_stats.stop_resolution_count),
            );
            checkpoint.insert(
                "semantic_agent_pending_obligation_count".to_string(),
                serde_json::Value::from(next_stats.pending_obligation_count),
            );
            checkpoint.insert(
                "semantic_pending_queue_count".to_string(),
                serde_json::Value::from(next_stats.pending_queue_count),
            );
            checkpoint.insert(
                "semantic_pending_permission_count".to_string(),
                serde_json::Value::from(next_stats.pending_permission_count),
            );
            checkpoint.insert(
                "semantic_titled_session_count".to_string(),
                serde_json::Value::from(next_stats.titled_session_count),
            );
            transaction.execute(
                "UPDATE local_store_migrations
                 SET checkpoint = ?2, revision = revision + 1, commit_id = ?3
                 WHERE migration_id = ?1",
                params![
                    migration_id,
                    serde_json::Value::Object(checkpoint.clone()).to_string(),
                    commit_id,
                ],
            )?;
            transaction.commit()?;
            stats = next_stats;
            after_session_id = Some(session_id);
            at_checkpoint(connection);
        }
    }
    Ok(stats)
}

struct SemanticEventChunk<'a> {
    migration_id: &'a str,
    source_kind: &'a str,
    source_ordinal: usize,
    source_path: &'a str,
    chunk_index: usize,
    next_event_ordinal: usize,
    next_record_ordinal: u64,
    next_source_byte_offset: Option<u64>,
    record_count: usize,
    stream_id: &'a StreamId,
    events: &'a [(LocalDomainEvent, i64)],
}

fn import_semantic_event_chunk(
    connection: &mut Connection,
    registry: &EventCodecRegistry,
    generation_id: &str,
    chunk: SemanticEventChunk<'_>,
    now_ms: i64,
) -> Result<i64, rusqlite::Error> {
    let SemanticEventChunk {
        migration_id,
        source_kind,
        source_ordinal,
        source_path,
        chunk_index,
        next_event_ordinal,
        next_record_ordinal,
        next_source_byte_offset,
        record_count,
        stream_id,
        events,
    } = chunk;
    if record_count == 0 {
        return Ok(0);
    }
    if events.is_empty() {
        let transaction = connection.transaction()?;
        let current_checkpoint: String = transaction.query_row(
            "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
            params![migration_id],
            |row| row.get(0),
        )?;
        let mut checkpoint: serde_json::Value = serde_json::from_str(&current_checkpoint)
            .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
        let checkpoint = checkpoint
            .as_object_mut()
            .ok_or_else(|| migration_collision("legacy semantic checkpoint is not an object"))?;
        checkpoint.insert(
            "substep".to_string(),
            serde_json::Value::String("semantic_events".to_string()),
        );
        checkpoint.insert(
            "semantic_source_kind".to_string(),
            serde_json::Value::String(source_kind.to_string()),
        );
        checkpoint.insert(
            "semantic_source_ordinal".to_string(),
            serde_json::Value::from(source_ordinal),
        );
        checkpoint.insert(
            "semantic_source_path".to_string(),
            serde_json::Value::String(source_path.to_string()),
        );
        checkpoint.insert(
            "semantic_next_record_ordinal".to_string(),
            serde_json::Value::from(next_record_ordinal),
        );
        checkpoint.insert(
            "semantic_next_event_ordinal".to_string(),
            serde_json::Value::from(next_event_ordinal),
        );
        checkpoint.insert(
            "semantic_next_chunk_index".to_string(),
            serde_json::Value::from(chunk_index.saturating_add(1)),
        );
        if let Some(next_source_byte_offset) = next_source_byte_offset {
            checkpoint.insert(
                "semantic_source_byte_offset".to_string(),
                serde_json::Value::from(next_source_byte_offset),
            );
            checkpoint.insert(
                "source_byte_offset".to_string(),
                serde_json::Value::from(next_source_byte_offset),
            );
        }
        checkpoint.insert(
            "semantic_chunk_record_count".to_string(),
            serde_json::Value::from(record_count),
        );
        checkpoint.insert(
            "semantic_chunk_event_count".to_string(),
            serde_json::Value::from(0),
        );
        checkpoint.insert(
            "semantic_chunk_decoded_bytes".to_string(),
            serde_json::Value::from(0),
        );
        transaction.execute(
            "UPDATE local_store_migrations
             SET phase = 'importing', checkpoint = ?2, revision = revision + 1
             WHERE migration_id = ?1",
            params![
                migration_id,
                serde_json::Value::Object(checkpoint.clone()).to_string(),
            ],
        )?;
        transaction.commit()?;
        return Ok(0);
    }
    let mut encoded = Vec::with_capacity(events.len());
    let mut binding = Sha256::new();
    let mut decoded_bytes = 0usize;
    binding.update(source_path.as_bytes());
    binding.update((chunk_index as u64).to_be_bytes());
    for (event, occurred_at_ms) in events {
        let payload = registry.encode(event).map_err(|error| {
            migration_collision(&format!("semantic event encode failed: {error}"))
        })?;
        binding.update((payload.event_type.len() as u32).to_be_bytes());
        binding.update(payload.event_type.as_bytes());
        binding.update(payload.payload_version.to_be_bytes());
        binding.update((payload.payload.len() as u64).to_be_bytes());
        binding.update(&payload.payload);
        binding.update(occurred_at_ms.to_be_bytes());
        decoded_bytes = decoded_bytes
            .checked_add(payload.payload.len().saturating_add(128))
            .ok_or_else(|| migration_collision("legacy semantic event byte count overflow"))?;
        if decoded_bytes > IMPORT_BYTE_LIMIT {
            return Err(migration_collision(
                "legacy semantic event chunk exceeds 16 MiB",
            ));
        }
        encoded.push((payload, *occurred_at_ms));
    }
    let payload_hash: [u8; 32] = binding.finalize().into();
    let digest = hex::encode(payload_hash);
    let commit_id = format!("migration-event-{digest}");
    let existing: Option<(Vec<u8>, String)> = connection
        .query_row(
            "SELECT payload_hash, state FROM logical_commits WHERE commit_id = ?1",
            params![commit_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((saved_hash, state)) = existing {
        if saved_hash.as_slice() != payload_hash.as_slice() || state != "sealed" {
            return Err(migration_collision(
                "legacy semantic event commit identity collision",
            ));
        }
        return i64::try_from(events.len())
            .map_err(|_| migration_collision("legacy semantic event count overflow"));
    }

    let transaction = connection.transaction()?;
    let current_head: i64 = transaction
        .query_row(
            "SELECT head FROM stream_heads WHERE stream_id = ?1",
            params![stream_id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let next_global: i64 = transaction.query_row(
        "SELECT next_global_sequence FROM store_metadata WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let event_count = i64::try_from(encoded.len())
        .map_err(|_| migration_collision("legacy semantic event count overflow"))?;
    let last_global = next_global
        .checked_add(event_count - 1)
        .ok_or_else(|| migration_collision("legacy semantic global sequence exhausted"))?;
    let new_head = current_head
        .checked_add(event_count)
        .ok_or_else(|| migration_collision("legacy semantic stream sequence exhausted"))?;
    transaction.execute(
        "INSERT INTO logical_commits (
            commit_id, generation_id, operation_kind, idempotency_key, payload_hash,
            state, first_global_sequence, last_global_sequence, event_count,
            mutation_count, stream_heads_json, result_hash, committed_at_ms
         ) VALUES (?1, ?2, 'migration', ?3, ?4, 'preparing', ?5, ?6, ?7,
                   1, ?8, NULL, ?9)",
        params![
            commit_id,
            generation_id,
            digest,
            payload_hash.as_slice(),
            next_global,
            last_global,
            event_count,
            serde_json::to_string(&vec![(stream_id.as_str(), new_head)])
                .map_err(|_| migration_collision("legacy semantic head encode failed"))?,
            now_ms,
        ],
    )?;
    for (offset, (payload, occurred_at_ms)) in encoded.into_iter().enumerate() {
        let offset = i64::try_from(offset)
            .map_err(|_| migration_collision("legacy semantic event ordinal overflow"))?;
        let global = next_global + offset;
        let sequence = current_head + offset + 1;
        let payload_sha256: [u8; 32] = Sha256::digest(&payload.payload).into();
        transaction.execute(
            "INSERT INTO events (
                global_sequence, event_id, commit_id, stream_id, stream_sequence,
                event_type, payload_version, occurred_at, payload, payload_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                global,
                format!("{commit_id}.{global}"),
                commit_id,
                stream_id.as_str(),
                sequence,
                payload.event_type,
                payload.payload_version,
                occurred_at_ms.to_string(),
                payload.payload,
                payload_sha256.as_slice(),
            ],
        )?;
    }
    transaction.execute(
        "INSERT INTO stream_heads (stream_id, head, updated_commit_id)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(stream_id) DO UPDATE SET
            head = excluded.head,
            updated_commit_id = excluded.updated_commit_id",
        params![stream_id.as_str(), new_head, commit_id],
    )?;
    transaction.execute(
        "UPDATE store_metadata SET next_global_sequence = ?1 WHERE id = 1",
        params![last_global + 1],
    )?;
    let current_checkpoint: String = transaction.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let mut checkpoint: serde_json::Value = serde_json::from_str(&current_checkpoint)
        .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
    let checkpoint = checkpoint
        .as_object_mut()
        .ok_or_else(|| migration_collision("legacy semantic checkpoint is not an object"))?;
    checkpoint.insert(
        "substep".to_string(),
        serde_json::Value::String("semantic_events".to_string()),
    );
    checkpoint.insert(
        "semantic_source_kind".to_string(),
        serde_json::Value::String(source_kind.to_string()),
    );
    checkpoint.insert(
        "semantic_source_ordinal".to_string(),
        serde_json::Value::from(source_ordinal),
    );
    checkpoint.insert(
        "semantic_source_path".to_string(),
        serde_json::Value::String(source_path.to_string()),
    );
    checkpoint.insert(
        "semantic_next_record_ordinal".to_string(),
        serde_json::Value::from(next_record_ordinal),
    );
    checkpoint.insert(
        "semantic_next_event_ordinal".to_string(),
        serde_json::Value::from(next_event_ordinal),
    );
    checkpoint.insert(
        "semantic_next_chunk_index".to_string(),
        serde_json::Value::from(chunk_index.saturating_add(1)),
    );
    if let Some(next_source_byte_offset) = next_source_byte_offset {
        checkpoint.insert(
            "semantic_source_byte_offset".to_string(),
            serde_json::Value::from(next_source_byte_offset),
        );
        checkpoint.insert(
            "source_byte_offset".to_string(),
            serde_json::Value::from(next_source_byte_offset),
        );
    }
    checkpoint.insert(
        "semantic_chunk_record_count".to_string(),
        serde_json::Value::from(record_count),
    );
    checkpoint.insert(
        "semantic_chunk_event_count".to_string(),
        serde_json::Value::from(events.len()),
    );
    checkpoint.insert(
        "semantic_chunk_decoded_bytes".to_string(),
        serde_json::Value::from(decoded_bytes),
    );
    transaction.execute(
        "UPDATE local_store_migrations
         SET phase = 'importing', checkpoint = ?2, revision = revision + 1,
             commit_id = ?3 WHERE migration_id = ?1",
        params![
            migration_id,
            serde_json::Value::Object(checkpoint.clone()).to_string(),
            commit_id,
        ],
    )?;
    let result_hash: [u8; 32] =
        Sha256::digest([commit_id.as_bytes(), payload_hash.as_slice()].concat()).into();
    transaction.execute(
        "UPDATE logical_commits SET state = 'sealed', result_hash = ?2
         WHERE commit_id = ?1",
        params![commit_id, result_hash.as_slice()],
    )?;
    transaction.commit()?;
    Ok(event_count)
}

const SEMANTIC_EVENT_CHUNK_LIMIT: usize = 4096;

struct SemanticEventImporter<'a, F> {
    connection: &'a mut Connection,
    registry: &'a EventCodecRegistry,
    generation_id: &'a str,
    migration_id: &'a str,
    source_kind: &'a str,
    source_ordinal: usize,
    source_path: &'a str,
    stream_id: &'a StreamId,
    now_ms: i64,
    at_checkpoint: &'a mut F,
    suppress_through_record_ordinal: u64,
    resume_chunk_index: usize,
    resume_source_byte_offset: Option<u64>,
    chunk_index: usize,
    next_record_ordinal: u64,
    next_event_ordinal: usize,
    next_source_byte_offset: Option<u64>,
    chunk_record_count: usize,
    chunk_decoded_bytes: usize,
    chunk_events: Vec<(LocalDomainEvent, i64)>,
    imported_event_count: i64,
}

impl<F> SemanticEventImporter<'_, F>
where
    F: FnMut(&Connection),
{
    fn next_event_ordinal(&self) -> Result<u64, rusqlite::Error> {
        u64::try_from(self.next_event_ordinal)
            .map_err(|_| migration_collision("legacy semantic event ordinal overflow"))
    }

    fn flush(&mut self) -> Result<(), rusqlite::Error> {
        if self.chunk_record_count == 0 {
            return Ok(());
        }
        let prefix_was_committed = self.next_record_ordinal <= self.suppress_through_record_ordinal;
        if prefix_was_committed && self.chunk_index >= self.resume_chunk_index {
            return Err(migration_collision(
                "legacy semantic checkpoint chunk index disagrees with its record ordinal",
            ));
        }
        if prefix_was_committed
            && self.next_record_ordinal == self.suppress_through_record_ordinal
            && self.resume_source_byte_offset.is_some()
            && self.next_source_byte_offset != self.resume_source_byte_offset
        {
            return Err(migration_collision(
                "legacy semantic checkpoint byte offset disagrees with its record ordinal",
            ));
        }
        let imported = if prefix_was_committed {
            i64::try_from(self.chunk_events.len())
                .map_err(|_| migration_collision("legacy semantic event count overflow"))?
        } else {
            if self.chunk_index < self.resume_chunk_index {
                return Err(migration_collision(
                    "legacy semantic checkpoint record ordinal disagrees with its chunk index",
                ));
            }
            import_semantic_event_chunk(
                self.connection,
                self.registry,
                self.generation_id,
                SemanticEventChunk {
                    migration_id: self.migration_id,
                    source_kind: self.source_kind,
                    source_ordinal: self.source_ordinal,
                    source_path: self.source_path,
                    chunk_index: self.chunk_index,
                    next_event_ordinal: self.next_event_ordinal,
                    next_record_ordinal: self.next_record_ordinal,
                    next_source_byte_offset: self.next_source_byte_offset,
                    record_count: self.chunk_record_count,
                    stream_id: self.stream_id,
                    events: &self.chunk_events,
                },
                self.now_ms,
            )?
        };
        self.imported_event_count = self
            .imported_event_count
            .checked_add(imported)
            .ok_or_else(|| migration_collision("legacy semantic event count overflow"))?;
        self.chunk_index = self
            .chunk_index
            .checked_add(1)
            .ok_or_else(|| migration_collision("legacy semantic chunk ordinal overflow"))?;
        self.chunk_record_count = 0;
        self.chunk_decoded_bytes = 0;
        self.chunk_events.clear();
        if !prefix_was_committed {
            (self.at_checkpoint)(self.connection);
        }
        Ok(())
    }

    fn push_record(
        &mut self,
        record_ordinal: u64,
        domain: Vec<(LocalDomainEvent, i64)>,
        next_source_byte_offset: Option<u64>,
    ) -> Result<(), rusqlite::Error> {
        if record_ordinal != self.next_record_ordinal {
            return Err(migration_collision(
                "legacy semantic record ordinal is not contiguous",
            ));
        }
        let event_count = domain.len();
        let mut record_decoded_bytes = 0usize;
        for (event, _) in &domain {
            let payload = self.registry.encode(event).map_err(|error| {
                migration_collision(&format!("semantic event encode failed: {error}"))
            })?;
            record_decoded_bytes = record_decoded_bytes
                .checked_add(payload.payload.len().saturating_add(128))
                .ok_or_else(|| migration_collision("legacy semantic event byte count overflow"))?;
        }
        if event_count > SEMANTIC_EVENT_CHUNK_LIMIT || record_decoded_bytes > IMPORT_BYTE_LIMIT {
            return Err(migration_collision(
                "one legacy semantic record exceeds the bounded import chunk",
            ));
        }
        let exceeds_current = self.chunk_record_count > 0
            && (self.chunk_record_count >= IMPORT_RECORD_LIMIT
                || self.chunk_events.len().saturating_add(event_count)
                    > SEMANTIC_EVENT_CHUNK_LIMIT
                || self
                    .chunk_decoded_bytes
                    .saturating_add(record_decoded_bytes)
                    > IMPORT_BYTE_LIMIT);
        if exceeds_current {
            self.flush()?;
        }
        self.chunk_events.extend(domain);
        self.chunk_record_count = self
            .chunk_record_count
            .checked_add(1)
            .ok_or_else(|| migration_collision("legacy semantic record count overflow"))?;
        self.chunk_decoded_bytes = self
            .chunk_decoded_bytes
            .checked_add(record_decoded_bytes)
            .ok_or_else(|| migration_collision("legacy semantic event byte count overflow"))?;
        self.next_record_ordinal = self
            .next_record_ordinal
            .checked_add(1)
            .ok_or_else(|| migration_collision("legacy semantic record ordinal overflow"))?;
        self.next_event_ordinal = self
            .next_event_ordinal
            .checked_add(event_count)
            .ok_or_else(|| migration_collision("legacy semantic event ordinal overflow"))?;
        self.next_source_byte_offset = next_source_byte_offset;
        if self.chunk_record_count >= IMPORT_RECORD_LIMIT
            || self.chunk_events.len() >= SEMANTIC_EVENT_CHUNK_LIMIT
            || self.chunk_decoded_bytes >= IMPORT_BYTE_LIMIT
        {
            self.flush()?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // Every import/checkpoint identity stays explicit.
fn import_agent_semantic_event_source<F>(
    connection: &mut Connection,
    registry: &EventCodecRegistry,
    generation_id: &str,
    migration_id: &str,
    app_data_root: &Path,
    record: &SourceRecord,
    source_ordinal: usize,
    stream_id: &StreamId,
    now_ms: i64,
    at_checkpoint: &mut F,
) -> Result<i64, rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let checkpoint: String = connection.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
    let substep = checkpoint
        .get("substep")
        .and_then(serde_json::Value::as_str);
    let saved_source_kind = checkpoint
        .get("semantic_source_kind")
        .and_then(serde_json::Value::as_str);
    let saved_source_ordinal = checkpoint
        .get("semantic_source_ordinal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let later_semantic_substep = matches!(
        substep,
        Some("semantic_session_events" | "semantic_session_projection" | "parity")
    ) || (substep == Some("semantic_events")
        && saved_source_kind == Some("workflow"));
    let (suppress_through_record_ordinal, resume_chunk_index) = if later_semantic_substep {
        (u64::MAX, usize::MAX)
    } else if substep == Some("semantic_events") && saved_source_kind == Some("agent_session") {
        match saved_source_ordinal {
            Some(saved) if source_ordinal < saved => (u64::MAX, usize::MAX),
            Some(saved) if source_ordinal == saved => (
                checkpoint
                    .get("semantic_next_record_ordinal")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                checkpoint
                    .get("semantic_next_chunk_index")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(0),
            ),
            _ => (0, 0),
        }
    } else {
        (0, 0)
    };
    let occurred_at_ms = i64::try_from(record.modified_ms).unwrap_or(i64::MAX);
    let mut importer = SemanticEventImporter {
        connection,
        registry,
        generation_id,
        migration_id,
        source_kind: "agent_session",
        source_ordinal,
        source_path: &record.relative_path,
        stream_id,
        now_ms,
        at_checkpoint,
        suppress_through_record_ordinal,
        resume_chunk_index,
        resume_source_byte_offset: None,
        chunk_index: 0,
        next_record_ordinal: 0,
        next_event_ordinal: 0,
        next_source_byte_offset: None,
        chunk_record_count: 0,
        chunk_decoded_bytes: 0,
        chunk_events: Vec::with_capacity(IMPORT_RECORD_LIMIT),
        imported_event_count: 0,
    };
    stream_inventoried_json_array(app_data_root, record, |record_ordinal, raw| {
        let event_ordinal = importer
            .next_event_ordinal()
            .map_err(|error| error.to_string())?;
        let decoded = crate::adaptor::gateway::agent_session::session_storage::decode_legacy_agent_event_record_v1(
            raw,
            &record.relative_path,
            event_ordinal,
        )?;
        let domain = decoded
            .into_iter()
            .map(|event| (LocalDomainEvent::AgentSession(event), occurred_at_ms))
            .collect();
        importer
            .push_record(record_ordinal, domain, None)
            .map_err(|error| error.to_string())
    })
    .map_err(|reason| migration_collision(&reason))?;
    importer.flush()?;
    Ok(importer.imported_event_count)
}

fn workflow_semantic_resume_position(
    checkpoint: &serde_json::Value,
    source_ordinal: usize,
) -> (u64, usize) {
    let substep = checkpoint
        .get("substep")
        .and_then(serde_json::Value::as_str);
    if substep == Some("parity") {
        return (u64::MAX, usize::MAX);
    }
    if substep != Some("semantic_events")
        || checkpoint
            .get("semantic_source_kind")
            .and_then(serde_json::Value::as_str)
            != Some("workflow")
    {
        return (0, 0);
    }
    let saved_source_ordinal = checkpoint
        .get("semantic_source_ordinal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    match saved_source_ordinal {
        Some(saved) if source_ordinal < saved => (u64::MAX, usize::MAX),
        Some(saved) if source_ordinal == saved => (
            checkpoint
                .get("semantic_next_record_ordinal")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            checkpoint
                .get("semantic_next_chunk_index")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0),
        ),
        _ => (0, 0),
    }
}

#[allow(clippy::too_many_arguments)] // Every import/checkpoint identity stays explicit.
fn import_workflow_semantic_event_source<F>(
    connection: &mut Connection,
    registry: &EventCodecRegistry,
    generation_id: &str,
    migration_id: &str,
    app_data_root: &Path,
    record: &SourceRecord,
    source_ordinal: usize,
    stream_id: &StreamId,
    now_ms: i64,
    at_checkpoint: &mut F,
) -> Result<i64, rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let checkpoint: String = connection.query_row(
        "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("legacy semantic checkpoint is invalid"))?;
    let (suppress_through_record_ordinal, resume_chunk_index) =
        workflow_semantic_resume_position(&checkpoint, source_ordinal);
    let resume_source_byte_offset = (suppress_through_record_ordinal != 0
        && suppress_through_record_ordinal != u64::MAX)
        .then(|| {
            checkpoint
                .get("semantic_source_byte_offset")
                .and_then(serde_json::Value::as_u64)
        })
        .flatten();
    let mut importer = SemanticEventImporter {
        connection,
        registry,
        generation_id,
        migration_id,
        source_kind: "workflow",
        source_ordinal,
        source_path: &record.relative_path,
        stream_id,
        now_ms,
        at_checkpoint,
        suppress_through_record_ordinal,
        resume_chunk_index,
        resume_source_byte_offset,
        chunk_index: 0,
        next_record_ordinal: 0,
        next_event_ordinal: 0,
        next_source_byte_offset: None,
        chunk_record_count: 0,
        chunk_decoded_bytes: 0,
        chunk_events: Vec::with_capacity(IMPORT_RECORD_LIMIT),
        imported_event_count: 0,
    };
    stream_inventoried_ndjson(
        app_data_root,
        record,
        |record_ordinal, next_source_byte_offset, raw| {
            let (event, occurred_at_ms) =
                crate::adaptor::gateway::workflow::log::decode_legacy_workflow_event_record_v1(
                    raw,
                    &record.relative_path,
                    record_ordinal,
                )?;
            importer
                .push_record(
                    record_ordinal,
                    vec![(LocalDomainEvent::Workflow(event), occurred_at_ms)],
                    Some(next_source_byte_offset),
                )
                .map_err(|error| error.to_string())
        },
    )
    .map_err(|reason| migration_collision(&reason))?;
    importer.flush()?;
    Ok(importer.imported_event_count)
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        files.push(path.to_path_buf());
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        collect_files(&entry.path(), files)?;
    }
    Ok(())
}

fn inventory_with_checkpoint<F>(
    app_data_root: &Path,
    mut at_checkpoint: F,
) -> Result<Vec<SourceRecord>, std::io::Error>
where
    F: FnMut(),
{
    let mut files = Vec::new();
    for root in SOURCE_ROOTS {
        collect_files(&app_data_root.join(root), &mut files)?;
    }
    files.sort_by(|left, right| {
        left.strip_prefix(app_data_root)
            .unwrap_or(left)
            .cmp(right.strip_prefix(app_data_root).unwrap_or(right))
    });
    files
        .into_iter()
        .map(|path| {
            let before = std::fs::metadata(&path)?;
            let sha256 = hash_source_with_checkpoint(&path, &mut at_checkpoint)?;
            let after = std::fs::metadata(&path)?;
            let before_modified = source_modified_ms(&before)?;
            let after_modified = source_modified_ms(&after)?;
            if before.len() != after.len() || before_modified != after_modified {
                return Err(std::io::Error::other(
                    "legacy source changed during inventory",
                ));
            }
            Ok(SourceRecord {
                relative_path: path
                    .strip_prefix(app_data_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                size: after.len(),
                modified_ms: after_modified,
                sha256,
            })
        })
        .collect()
}

#[cfg(test)]
fn inventory(app_data_root: &Path) -> Result<Vec<SourceRecord>, std::io::Error> {
    inventory_with_checkpoint(app_data_root, || {})
}

fn inventory_hash(records: &[SourceRecord]) -> [u8; 32] {
    let mut inventory_hasher = Sha256::new();
    for record in records {
        inventory_hasher.update((record.relative_path.len() as u32).to_be_bytes());
        inventory_hasher.update(record.relative_path.as_bytes());
        inventory_hasher.update(record.size.to_be_bytes());
        inventory_hasher.update(record.modified_ms.to_be_bytes());
        inventory_hasher.update(record.sha256);
    }
    inventory_hasher.finalize().into()
}

fn verify_saved_inventory(
    connection: &Connection,
    migration_id: &str,
    expected: &[SourceRecord],
) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT source_ordinal, source_path, source_size, modified_ms,
                raw_sha256, record_count
         FROM legacy_source_inventory
         WHERE migration_id = ?1 ORDER BY source_ordinal ASC",
    )?;
    let rows = statement.query_map(params![migration_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut saved_count = 0usize;
    for (position, row) in rows.enumerate() {
        let (ordinal, path, size, modified_ms, sha256, record_count) = row?;
        let Some(record) = expected.get(position) else {
            return Err(migration_collision(
                "legacy source inventory contains an unexpected row",
            ));
        };
        let saved_sha256: [u8; 32] = sha256.try_into().map_err(|_| {
            migration_collision("legacy source inventory hash has an invalid length")
        })?;
        if ordinal != i64::try_from(position).unwrap_or(i64::MAX)
            || path != record.relative_path
            || size.parse::<u64>().ok() != Some(record.size)
            || modified_ms.parse::<u128>().ok() != Some(record.modified_ms)
            || saved_sha256 != record.sha256
            || record_count != 1
        {
            return Err(migration_collision(
                "legacy source inventory no longer matches the fixed source",
            ));
        }
        saved_count = saved_count.saturating_add(1);
    }
    if saved_count != expected.len() {
        return Err(migration_collision(
            "legacy source inventory row count is incomplete",
        ));
    }
    Ok(())
}

fn persist_source_inventory<F>(
    connection: &mut Connection,
    migration_id: &str,
    commit_id: &str,
    records: &[SourceRecord],
    at_checkpoint: &mut F,
) -> Result<(), rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let mut start = 0usize;
    while start < records.len() {
        let end = records.len().min(start.saturating_add(IMPORT_RECORD_LIMIT));
        let transaction = connection.transaction()?;
        for (offset, record) in records[start..end].iter().enumerate() {
            let ordinal = i64::try_from(start.saturating_add(offset))
                .map_err(|_| migration_collision("legacy source inventory ordinal overflow"))?;
            transaction.execute(
                "INSERT OR IGNORE INTO legacy_source_inventory
                    (migration_id, source_ordinal, source_path, source_size,
                     modified_ms, raw_sha256, record_count, commit_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
                params![
                    migration_id,
                    ordinal,
                    record.relative_path,
                    record.size.to_string(),
                    record.modified_ms.to_string(),
                    record.sha256.as_slice(),
                    commit_id,
                ],
            )?;
        }
        let checkpoint = serde_json::json!({
            "schema": "legacy_migration_checkpoint_v1",
            "substep": "inventory_persist",
            "inventory_next_source_ordinal": end,
            "total_source_count": records.len(),
            "read_only": true,
        })
        .to_string();
        transaction.execute(
            "UPDATE local_store_migrations
             SET phase = 'inspecting_source', checkpoint = ?2,
                 revision = revision + 1, commit_id = ?3
             WHERE migration_id = ?1",
            params![migration_id, checkpoint, commit_id],
        )?;
        transaction.commit()?;
        start = end;
        at_checkpoint(connection);
    }
    verify_saved_inventory(connection, migration_id, records)
}

pub(crate) fn verify_source_unchanged<F>(
    app_data_root: &Path,
    expected: [u8; 32],
    at_checkpoint: F,
) -> Result<(), std::io::Error>
where
    F: FnMut(),
{
    let actual = inventory_hash(&inventory_with_checkpoint(app_data_root, at_checkpoint)?);
    if actual != expected {
        return Err(std::io::Error::other(
            "legacy source changed after migration inventory",
        ));
    }
    Ok(())
}

pub(crate) fn legacy_source_exists(app_data_root: &Path) -> bool {
    SOURCE_ROOTS
        .iter()
        .any(|root| app_data_root.join(root).exists())
}

/// Ensure a Legacy pointer has an immutable staging locator before source
/// inspection begins. Returns the current pointer after any CAS.
pub(crate) fn prepare_authority(
    app_data_root: &Path,
    layout: &StoreLayout,
    current: Option<LocalStoreAuthorityPointerV1>,
) -> Result<Option<LocalStoreAuthorityPointerV1>, AuthorityError> {
    match current {
        None if legacy_source_exists(app_data_root) => {
            let pointer = LocalStoreAuthorityPointerV1::Legacy {
                source_generation_id: uuid::Uuid::new_v4().to_string(),
                migration: Some(AuthorityMigrationRef {
                    migration_id: uuid::Uuid::new_v4().to_string(),
                    staging_generation_id: uuid::Uuid::new_v4().to_string(),
                }),
            };
            cas_authority(layout, None, &pointer, None)?;
            Ok(Some(pointer))
        }
        Some(
            pointer @ LocalStoreAuthorityPointerV1::Legacy {
                migration: Some(_), ..
            },
        ) => Ok(Some(pointer)),
        Some(LocalStoreAuthorityPointerV1::Legacy {
            source_generation_id,
            migration: None,
        }) => {
            let pointer = LocalStoreAuthorityPointerV1::Legacy {
                source_generation_id: source_generation_id.clone(),
                migration: None,
            };
            let next = LocalStoreAuthorityPointerV1::Legacy {
                source_generation_id,
                migration: Some(AuthorityMigrationRef {
                    migration_id: uuid::Uuid::new_v4().to_string(),
                    staging_generation_id: uuid::Uuid::new_v4().to_string(),
                }),
            };
            cas_authority(layout, Some(&pointer), &next, None)?;
            Ok(Some(next))
        }
        other => Ok(other),
    }
}

/// Import the immutable inventory in bounded SQLite transactions and return
/// its canonical inventory hash. Raw records allow a future additive codec
/// to recover bytes without consulting the legacy authority again.
pub(crate) fn import_legacy<F>(
    connection: &mut Connection,
    app_data_root: &Path,
    migration_id: &str,
    generation_id: &str,
    now_ms: i64,
    mut at_checkpoint: F,
) -> Result<[u8; 32], rusqlite::Error>
where
    F: FnMut(&Connection),
{
    let commit_id = format!("migration-{migration_id}");
    let inspecting_binding: [u8; 32] =
        Sha256::digest(format!("legacy-migration-inspection/v1\0{migration_id}").as_bytes()).into();
    connection.execute(
        "INSERT OR IGNORE INTO logical_commits
            (commit_id, generation_id, operation_kind, idempotency_key,
             payload_hash, state, first_global_sequence, last_global_sequence,
             event_count, mutation_count, stream_heads_json, result_hash,
             committed_at_ms)
         VALUES (?1, ?2, 'migration', ?3, ?4, 'preparing', NULL, NULL,
                 0, 1, '{}', NULL, ?5)",
        params![
            commit_id,
            generation_id,
            format!("legacy-migration-{migration_id}"),
            inspecting_binding.as_slice(),
            now_ms,
        ],
    )?;
    let inspecting_checkpoint = serde_json::json!({
        "schema": "legacy_migration_checkpoint_v1",
        "substep": "inventory",
        "next_source_ordinal": 0,
        "total_source_count": 0,
        "read_only": true,
    })
    .to_string();
    connection.execute(
        "INSERT OR IGNORE INTO local_store_migrations
            (migration_id, phase, source_inventory_hash, checkpoint, parity,
             revision, commit_id)
         VALUES (?1, 'inspecting_source', zeroblob(32), ?2, NULL, 0, ?3)",
        params![migration_id, inspecting_checkpoint, commit_id],
    )?;
    // The migration row exists before filesystem inspection so a critical
    // migration-safe quit can bind to this exact staging database.
    at_checkpoint(connection);
    let records = inventory_with_checkpoint(app_data_root, || at_checkpoint(connection))
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let message_ordinals = legacy_message_ordinals(&records)?;
    let inventory_hash = inventory_hash(&records);
    let mut session_titles = std::collections::HashMap::new();
    for record in &records {
        if record.relative_path != "session_titles.json" {
            continue;
        }
        let raw = read_inventoried_source(app_data_root, record)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        if let Some(decoded) = crate::adaptor::gateway::agent_session::session_storage::decode_legacy_session_titles_v1(
            &record.relative_path,
            &raw,
        )
        .map_err(|reason| migration_collision(&reason))?
        {
            session_titles = decoded;
        }
    }
    let checkpoint = serde_json::json!({
        "schema": "legacy_migration_checkpoint_v1",
        "next_source_ordinal": 0,
        "total_source_count": records.len(),
        "read_only": true,
    })
    .to_string();
    let saved_hash: Vec<u8> = connection.query_row(
        "SELECT source_inventory_hash FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
        |row| row.get(0),
    )?;
    if saved_hash.iter().all(|byte| *byte == 0) {
        persist_source_inventory(
            connection,
            migration_id,
            &commit_id,
            &records,
            &mut at_checkpoint,
        )?;
        connection.execute(
            "UPDATE local_store_migrations
             SET phase = 'importing', source_inventory_hash = ?2,
                 checkpoint = ?3, revision = revision + 1, commit_id = ?4
             WHERE migration_id = ?1 AND phase = 'inspecting_source'",
            params![
                migration_id,
                inventory_hash.as_slice(),
                checkpoint,
                commit_id
            ],
        )?;
    } else {
        verify_saved_inventory(connection, migration_id, &records)?;
    }
    at_checkpoint(connection);
    let (saved_inventory, saved_checkpoint): (Vec<u8>, String) = connection.query_row(
        "SELECT source_inventory_hash, checkpoint FROM local_store_migrations
         WHERE migration_id = ?1",
        params![migration_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if saved_inventory.as_slice() != inventory_hash.as_slice() {
        return Err(migration_collision(
            "legacy migration inventory identity changed",
        ));
    }
    let checkpoint_value: serde_json::Value = serde_json::from_str(&saved_checkpoint)
        .map_err(|_| migration_collision("legacy migration checkpoint is invalid"))?;
    let mut start = checkpoint_value
        .get("next_source_ordinal")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| migration_collision("legacy migration checkpoint ordinal is invalid"))?;
    if start > records.len() {
        return Err(migration_collision(
            "legacy migration checkpoint exceeds source inventory",
        ));
    }
    let mut ordinal = checkpoint_value
        .get("imported_raw_record_count")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let mut semantic_session_count = checkpoint_value
        .get("semantic_session_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut semantic_message_count = checkpoint_value
        .get("semantic_message_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut semantic_workflow_count = checkpoint_value
        .get("semantic_workflow_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    while start < records.len() {
        if records[start].size > IMPORT_BYTE_LIMIT as u64 {
            import_chunked_raw_source(
                connection,
                app_data_root,
                &records[start],
                migration_id,
                &commit_id,
                start,
                ordinal,
                &mut at_checkpoint,
            )?;
            start += 1;
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| migration_collision("legacy raw record count overflow"))?;
            continue;
        }
        let mut end = start;
        let mut bytes = 0usize;
        while end < records.len()
            && end - start < IMPORT_RECORD_LIMIT
            && bytes.saturating_add(records[end].size as usize) <= IMPORT_BYTE_LIMIT
        {
            bytes = bytes.saturating_add(records[end].size as usize);
            end += 1;
        }
        debug_assert!(end > start);
        let transaction = connection.transaction()?;
        for record in &records[start..end] {
            let raw = read_inventoried_source(app_data_root, record)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            if let Some(decoded) = crate::adaptor::gateway::agent_session::session_storage::decode_legacy_session_projection_v1(
                &record.relative_path,
                &raw,
            )
            .map_err(|reason| migration_collision(&reason))?
            {
                use crate::adaptor::gateway::agent_session::session_storage::LegacySessionProjectionV1;
                match decoded {
                    LegacySessionProjectionV1::Session {
                        session_id,
                        projection,
                        messages,
                    } => {
                        insert_session_projection(
                            &transaction,
                            &session_id,
                            &projection,
                            &commit_id,
                        )?;
                        semantic_session_count += 1;
                        for (index, (message_id, projection)) in messages.into_iter().enumerate() {
                            let message_ordinal = i64::try_from(index)
                                .ok()
                                .and_then(|index| index.checked_add(1))
                                .ok_or_else(|| {
                                    migration_collision("legacy message ordinal overflow")
                                })?;
                            insert_message_projection(
                                &transaction,
                                &session_id,
                                &message_id,
                                message_ordinal,
                                &projection,
                                &commit_id,
                            )?;
                            semantic_message_count += 1;
                        }
                    }
                    LegacySessionProjectionV1::Message {
                        session_id,
                        message_id,
                        projection,
                    } => {
                        let message_ordinal = message_ordinals
                            .get(&record.relative_path)
                            .copied()
                            .ok_or_else(|| {
                                migration_collision("legacy message ordinal is missing")
                            })?;
                        insert_message_projection(
                            &transaction,
                            &session_id,
                            &message_id,
                            message_ordinal,
                            &projection,
                            &commit_id,
                        )?;
                        semantic_message_count += 1;
                    }
                }
            }
            if record.relative_path.ends_with("/private_context.json") {
                let components = record.relative_path.split('/').collect::<Vec<_>>();
                let ["sessions", session_id, "private_context.json"] = components.as_slice() else {
                    return Err(migration_collision(
                        "legacy private context path is invalid",
                    ));
                };
                let existing: Option<String> = transaction
                    .query_row(
                        "SELECT projection FROM session_projection WHERE session_id = ?1",
                        params![session_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let existing = existing.ok_or_else(|| {
                    migration_collision("legacy private context has no session projection")
                })?;
                let Some((decoded_session_id, merged)) = crate::adaptor::gateway::agent_session::session_storage::merge_legacy_private_context_projection_v1(
                    &record.relative_path,
                    &raw,
                    &existing,
                )
                .map_err(|reason| migration_collision(&reason))?
                else {
                    return Err(migration_collision("legacy private context is incompatible"));
                };
                if decoded_session_id != *session_id {
                    return Err(migration_collision(
                        "legacy private context session identity changed",
                    ));
                }
                replace_session_projection(
                    &transaction,
                    session_id,
                    &existing,
                    &merged,
                    &commit_id,
                )?;
            }
            if let Some((execution_id, projection)) =
                crate::adaptor::gateway::workflow::execution_store::decode_legacy_workflow_projection_v1(
                    &record.relative_path,
                    &raw,
                )
                .map_err(|reason| migration_collision(&reason))?
            {
                insert_session_projection(
                    &transaction,
                    &format!("workflow:{execution_id}"),
                    &projection,
                    &commit_id,
                )?;
                let pending = crate::adaptor::gateway::workflow::execution_store::workflow_projection_is_non_terminal(&projection)
                    .map_err(|reason| migration_collision(&reason))?;
                insert_workflow_execution_obligation(
                    &transaction,
                    &execution_id,
                    &projection,
                    pending,
                    &commit_id,
                )?;
                semantic_workflow_count += 1;
            }
            let segments = record.relative_path.split('/').collect::<Vec<_>>();
            if let ["sessions", session_id, "attachments", attachment_id] = segments.as_slice() {
                let media_type = crate::domain::agent_session::services::detect_image_mime(&raw)
                    .ok_or_else(|| {
                        migration_collision("legacy attachment bytes are unsupported")
                    })?;
                let mut identity = Sha256::new();
                identity.update(media_type.as_bytes());
                identity.update([0]);
                identity.update(&raw);
                if hex::encode(identity.finalize()) != *attachment_id {
                    return Err(migration_collision("legacy attachment identity collision"));
                }
                let projection = serde_json::json!({
                    "schema": "agent_content_blob_v1",
                    "kind": "attachment",
                    "id": attachment_id,
                    "media_type": media_type,
                    "byte_size": raw.len().to_string(),
                    "data_base64": base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &raw,
                    ),
                    "content": null,
                })
                .to_string();
                insert_message_projection(
                    &transaction,
                    &format!("blob:{session_id}"),
                    &format!("attachment:{attachment_id}"),
                    next_message_ordinal(&transaction, &format!("blob:{session_id}"))?,
                    &projection,
                    &commit_id,
                )?;
            } else if let ["sessions", session_id, "tool_outputs", tool_output_id] =
                segments.as_slice()
            {
                if hex::encode(Sha256::digest(&raw)) != *tool_output_id {
                    return Err(migration_collision("legacy tool output identity collision"));
                }
                let content = std::str::from_utf8(&raw)
                    .map_err(|_| migration_collision("legacy tool output is not UTF-8"))?;
                let projection = serde_json::json!({
                    "schema": "agent_content_blob_v1",
                    "kind": "tool_output",
                    "id": tool_output_id,
                    "media_type": null,
                    "byte_size": raw.len().to_string(),
                    "data_base64": null,
                    "content": content,
                })
                .to_string();
                insert_message_projection(
                    &transaction,
                    &format!("blob:{session_id}"),
                    &format!("tool_output:{tool_output_id}"),
                    next_message_ordinal(&transaction, &format!("blob:{session_id}"))?,
                    &projection,
                    &commit_id,
                )?;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO legacy_raw_records
                    (migration_id, source_ordinal, source_path, source_size,
                     modified_ms, record_count, raw, raw_sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
                params![
                    migration_id,
                    ordinal,
                    record.relative_path,
                    record.size.to_string(),
                    record.modified_ms.to_string(),
                    raw,
                    record.sha256.as_slice(),
                ],
            )?;
            ordinal += 1;
        }
        let checkpoint = serde_json::json!({
            "schema": "legacy_migration_checkpoint_v1",
            "next_source_ordinal": end,
            "total_source_count": records.len(),
            "imported_raw_record_count": ordinal,
            "semantic_session_count": semantic_session_count,
            "semantic_message_count": semantic_message_count,
            "semantic_workflow_count": semantic_workflow_count,
            "read_only": true,
        })
        .to_string();
        transaction.execute(
            "UPDATE local_store_migrations
             SET phase = 'importing', checkpoint = ?2, revision = revision + 1,
                 commit_id = ?3 WHERE migration_id = ?1",
            params![migration_id, checkpoint, commit_id],
        )?;
        transaction.commit()?;
        start = end;
        at_checkpoint(connection);
    }

    semantic_message_count = import_large_message_semantics(
        connection,
        app_data_root,
        &records,
        &message_ordinals,
        migration_id,
        &commit_id,
        semantic_message_count,
        &mut at_checkpoint,
    )?;

    let registry = EventCodecRegistry::new();
    let mut semantic_event_count = 0i64;
    let mut agent_event_sources = Vec::new();
    for (record_index, record) in records.iter().enumerate() {
        if let Some((session_id, order)) =
            crate::adaptor::gateway::agent_session::session_storage::legacy_agent_event_source_identity_v1(
                &record.relative_path,
            )
            .map_err(|reason| migration_collision(&reason))?
        {
            agent_event_sources.push((record_index, session_id, order));
        }
    }
    agent_event_sources.sort_by(
        |(_, left_session, left_order), (_, right_session, right_order)| {
            left_session
                .cmp(right_session)
                .then_with(|| left_order.cmp(right_order))
        },
    );
    for (record_index, session_id, _) in &agent_event_sources {
        let record_index = *record_index;
        let record = &records[record_index];
        let stream_id = StreamId::agent_session(session_id)
            .map_err(|_| migration_collision("legacy agent stream identity is invalid"))?;
        semantic_event_count =
            semantic_event_count.saturating_add(import_agent_semantic_event_source(
                connection,
                &registry,
                generation_id,
                migration_id,
                app_data_root,
                record,
                record_index,
                &stream_id,
                now_ms,
                &mut at_checkpoint,
            )?);
    }
    let agent_semantic_stats = materialize_legacy_agent_semantics(
        connection,
        app_data_root,
        &records,
        &agent_event_sources,
        &session_titles,
        migration_id,
        &commit_id,
        &mut at_checkpoint,
    )?;
    for (record_index, record) in records.iter().enumerate() {
        let Some(execution_id) =
            crate::adaptor::gateway::workflow::log::legacy_workflow_event_source_identity_v1(
                &record.relative_path,
            )
            .map_err(|reason| migration_collision(&reason))?
        else {
            continue;
        };
        let stream_id = StreamId::workflow(&execution_id)
            .map_err(|_| migration_collision("legacy workflow stream identity is invalid"))?;
        semantic_event_count =
            semantic_event_count.saturating_add(import_workflow_semantic_event_source(
                connection,
                &registry,
                generation_id,
                migration_id,
                app_data_root,
                record,
                record_index,
                &stream_id,
                now_ms,
                &mut at_checkpoint,
            )?);
    }

    let projection_field_parity = verify_semantic_projections(
        connection,
        semantic_session_count,
        semantic_message_count,
        semantic_workflow_count,
        agent_semantic_stats,
        |connection| at_checkpoint(connection),
    )?;
    at_checkpoint(connection);

    let semantic_session_after: Option<String> = connection.query_row(
        "SELECT MAX(session_id) FROM session_projection
         WHERE session_id NOT LIKE 'workflow:%'",
        [],
        |row| row.get(0),
    )?;

    let parity = serde_json::json!({
        "schema": "legacy_migration_parity_v1",
        "source_count": records.len(),
        "raw_record_count": ordinal,
        "semantic_session_count": semantic_session_count,
        "semantic_message_count": semantic_message_count,
        "semantic_workflow_count": semantic_workflow_count,
        "semantic_event_count": semantic_event_count,
        "semantic_terminal_count": agent_semantic_stats.terminal_count,
        "semantic_stop_resolution_count": agent_semantic_stats.stop_resolution_count,
        "semantic_agent_pending_obligation_count": agent_semantic_stats.pending_obligation_count,
        "semantic_pending_queue_count": agent_semantic_stats.pending_queue_count,
        "semantic_pending_permission_count": agent_semantic_stats.pending_permission_count,
        "semantic_titled_session_count": agent_semantic_stats.titled_session_count,
        "semantic_workflow_instruction_count": projection_field_parity.workflow_instruction_count,
        "semantic_workflow_instruction_sha256": hex::encode(projection_field_parity.workflow_instruction_sha256),
        "semantic_context_epoch_payload_count": projection_field_parity.context_epoch_payload_count,
        "semantic_context_epoch_payload_sha256": hex::encode(projection_field_parity.context_epoch_payload_sha256),
        "semantic_agent_read_path_count": projection_field_parity.agent_read_path_count,
        "semantic_agent_read_path_sha256": hex::encode(projection_field_parity.agent_read_path_sha256),
        "semantic_owner_relation_count": projection_field_parity.owner_relation_count,
        "semantic_owner_relation_sha256": hex::encode(projection_field_parity.owner_relation_sha256),
        "inventory_sha256": hex::encode(inventory_hash),
        "source_unchanged": true,
        "integrity": "pending",
    })
    .to_string();
    let verifying_checkpoint = serde_json::json!({
        "schema": "legacy_migration_checkpoint_v1",
        "substep": "parity",
        "next_source_ordinal": records.len(),
        "total_source_count": records.len(),
        "imported_raw_record_count": ordinal,
        "semantic_session_count": semantic_session_count,
        "semantic_message_count": semantic_message_count,
        "semantic_workflow_count": semantic_workflow_count,
        "semantic_event_count": semantic_event_count,
        "semantic_session_after": semantic_session_after,
        "semantic_terminal_count": agent_semantic_stats.terminal_count,
        "semantic_stop_resolution_count": agent_semantic_stats.stop_resolution_count,
        "semantic_agent_pending_obligation_count": agent_semantic_stats.pending_obligation_count,
        "semantic_pending_queue_count": agent_semantic_stats.pending_queue_count,
        "semantic_pending_permission_count": agent_semantic_stats.pending_permission_count,
        "semantic_titled_session_count": agent_semantic_stats.titled_session_count,
        "semantic_workflow_instruction_count": projection_field_parity.workflow_instruction_count,
        "semantic_context_epoch_payload_count": projection_field_parity.context_epoch_payload_count,
        "semantic_agent_read_path_count": projection_field_parity.agent_read_path_count,
        "semantic_owner_relation_count": projection_field_parity.owner_relation_count,
        "read_only": true,
    })
    .to_string();
    connection.execute(
        "UPDATE logical_commits
         SET mutation_count = ?2
         WHERE commit_id = ?1 AND operation_kind = 'migration'",
        params![
            commit_id,
            1_i64
                .saturating_add(i64::try_from(records.len()).unwrap_or(i64::MAX))
                .saturating_add(ordinal)
                .saturating_add(i64::try_from(semantic_session_count).unwrap_or(i64::MAX))
                .saturating_add(i64::try_from(semantic_message_count).unwrap_or(i64::MAX))
                .saturating_add(i64::try_from(semantic_workflow_count).unwrap_or(i64::MAX))
                .saturating_add(
                    i64::try_from(agent_semantic_stats.terminal_count).unwrap_or(i64::MAX),
                )
                .saturating_add(
                    i64::try_from(agent_semantic_stats.stop_resolution_count).unwrap_or(i64::MAX),
                )
                .saturating_add(
                    i64::try_from(agent_semantic_stats.pending_obligation_count)
                        .unwrap_or(i64::MAX),
                ),
        ],
    )?;
    connection.execute(
        "UPDATE local_store_migrations
         SET phase = 'verifying', checkpoint = ?2, parity = ?3,
             revision = revision + 1, commit_id = ?4 WHERE migration_id = ?1",
        params![migration_id, verifying_checkpoint, parity, commit_id],
    )?;
    Ok(inventory_hash)
}

pub(crate) fn mark_activating(
    connection: &mut Connection,
    migration_id: &str,
    boot_id: &str,
    verified_source_inventory_hash: [u8; 32],
) -> Result<(), rusqlite::Error> {
    let transaction = connection.transaction()?;
    let (parity, checkpoint, stored_inventory_hash, migration_revision, commit_id): (
        String,
        String,
        Vec<u8>,
        i64,
        String,
    ) = transaction.query_row(
        "SELECT parity, checkpoint, source_inventory_hash, revision, commit_id
         FROM local_store_migrations
         WHERE migration_id = ?1 AND phase = 'verifying'",
        params![migration_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let mut parity: serde_json::Value = serde_json::from_str(&parity)
        .map_err(|_| migration_collision("migration parity is invalid"))?;
    let expected_inventory_sha256 = hex::encode(&stored_inventory_hash);
    if stored_inventory_hash.as_slice() != verified_source_inventory_hash.as_slice()
        || parity.get("schema").and_then(serde_json::Value::as_str)
            != Some("legacy_migration_parity_v1")
        || parity
            .get("source_unchanged")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || parity.get("integrity").and_then(serde_json::Value::as_str) != Some("pending")
        || parity
            .get("inventory_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(expected_inventory_sha256.as_str())
    {
        return Err(migration_collision(
            "migration parity is not ready for activation",
        ));
    }
    let (session_count, message_count, workflow_count, event_count, agent_stats) =
        semantic_parity_counts(&parity)?;
    let expected_projection_field_parity = semantic_projection_field_parity(&parity)?;
    let checkpoint_value: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("migration checkpoint is invalid"))?;
    if checkpoint_value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some("legacy_migration_checkpoint_v1")
        || checkpoint_value
            .get("substep")
            .and_then(serde_json::Value::as_str)
            != Some("parity")
        || checkpoint_value
            .get("read_only")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(migration_collision(
            "migration checkpoint is not ready for activation",
        ));
    }
    let source_inventory_hash = verified_source_inventory_hash;
    let (generation_id, operation_kind, idempotency_key, state, event_commit_count, mutation_count): (
        String,
        String,
        String,
        String,
        i64,
        i64,
    ) = transaction.query_row(
        "SELECT generation_id, operation_kind, idempotency_key, state,
                event_count, mutation_count
         FROM logical_commits WHERE commit_id = ?1",
        params![commit_id],
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
    )?;
    if operation_kind != "migration"
        || idempotency_key != format!("legacy-migration-{migration_id}")
        || state != "preparing"
        || event_commit_count != 0
    {
        return Err(migration_collision(
            "migration logical commit is not ready to seal",
        ));
    }

    // This is the last full read of mutable semantic state. It happens before
    // the authority pointer can become SQLite and remains inside the writer's
    // activation boundary.
    let actual_projection_field_parity = verify_semantic_projections(
        &transaction,
        session_count,
        message_count,
        workflow_count,
        agent_stats,
        |_| {},
    )?;
    if actual_projection_field_parity != expected_projection_field_parity {
        return Err(migration_collision(
            "migration semantic projection fields changed before activation",
        ));
    }
    if stored_semantic_event_count(&transaction)? != event_count {
        return Err(migration_collision(
            "migration semantic event parity changed before activation",
        ));
    }

    let parity_object = parity
        .as_object_mut()
        .ok_or_else(|| migration_collision("migration parity is not an object"))?;
    parity_object.insert(
        "integrity".to_string(),
        serde_json::Value::String("ok".to_string()),
    );
    let mut checkpoint = checkpoint_value;
    let checkpoint_object = checkpoint
        .as_object_mut()
        .ok_or_else(|| migration_collision("migration checkpoint is not an object"))?;
    checkpoint_object.insert(
        "substep".to_string(),
        serde_json::Value::String("authority_pointer".to_string()),
    );
    let final_parity = parity.to_string();
    let final_checkpoint = checkpoint.to_string();
    let final_revision = migration_revision
        .checked_add(1)
        .ok_or_else(|| migration_collision("migration revision overflow"))?;
    let activation_proof = activation_proof_digest(
        migration_id,
        &commit_id,
        &generation_id,
        final_revision,
        mutation_count,
        &source_inventory_hash,
        &final_checkpoint,
        &final_parity,
    );

    transaction.execute(
        "UPDATE store_metadata SET health = 'ok', boot_id = ?1 WHERE id = 1",
        params![boot_id],
    )?;
    let sealed = transaction.execute(
        "UPDATE logical_commits
         SET payload_hash = ?2, state = 'sealed', result_hash = ?3
         WHERE commit_id = ?1 AND operation_kind = 'migration'
           AND state = 'preparing' AND event_count = 0",
        params![
            commit_id,
            source_inventory_hash.as_slice(),
            activation_proof.as_slice(),
        ],
    )?;
    if sealed != 1 {
        return Err(migration_collision(
            "migration activation proof compare-and-set failed",
        ));
    }
    let changed = transaction.execute(
        "UPDATE local_store_migrations
         SET phase = 'activating', parity = ?2, checkpoint = ?3,
             revision = ?4
         WHERE migration_id = ?1 AND phase = 'verifying' AND revision = ?5
           AND commit_id = ?6",
        params![
            migration_id,
            final_parity,
            final_checkpoint,
            final_revision,
            migration_revision,
            commit_id,
        ],
    )?;
    if changed != 1 {
        return Err(migration_collision(
            "migration activation phase compare-and-set failed",
        ));
    }
    transaction.commit()?;
    Ok(())
}

/// A process may stop after the activation proof commits but before the
/// external authority-pointer CAS. On the next boot the pointer is still
/// Legacy, while this immutable row proves import must not run again.
pub(crate) fn activated_migration_inventory_hash(
    connection: &Connection,
    migration_id: &str,
) -> Result<Option<[u8; 32]>, rusqlite::Error> {
    let saved: Option<Vec<u8>> = connection
        .query_row(
            "SELECT source_inventory_hash FROM local_store_migrations
             WHERE migration_id = ?1 AND phase = 'activating'",
            params![migration_id],
            |row| row.get(0),
        )
        .optional()?;
    saved
        .map(|raw| {
            raw.try_into().map_err(|_| {
                migration_collision("activated migration inventory hash has an invalid length")
            })
        })
        .transpose()
}

const ACTIVATION_PROOF_MAX_JSON_BYTES: usize = 64 * 1024;

struct ActivatedMigrationProof {
    source_inventory_hash: [u8; 32],
    source_count: u64,
    raw_record_count: u64,
    semantic_event_count: u64,
    agent_semantic_stats: AgentSemanticMigrationStats,
    commit_id: String,
}

/// Validate the immutable, point-addressed attestation sealed immediately
/// before the authority-pointer CAS. This deliberately does not rescan the
/// source inventory or raw bytes: the exclusive writer performs that full
/// verification at startup/cutover, while short-lived read-only CLI
/// processes need an O(1) authority proof before a bounded projection query.
#[allow(clippy::type_complexity)] // Mirrors one fixed SQLite proof-row schema for exact validation.
fn load_activated_migration_proof(
    connection: &Connection,
    migration_id: &str,
) -> Result<ActivatedMigrationProof, rusqlite::Error> {
    let (phase, stored_inventory_hash, checkpoint, parity, migration_revision, commit_id): (
        String,
        Vec<u8>,
        String,
        Option<String>,
        i64,
        String,
    ) = connection.query_row(
        "SELECT phase, source_inventory_hash, checkpoint, parity, revision, commit_id
             FROM local_store_migrations WHERE migration_id = ?1",
        params![migration_id],
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
    )?;
    if phase != "activating" {
        return Err(migration_collision(
            "activated authority does not reference an activating migration",
        ));
    }
    if checkpoint.len() > ACTIVATION_PROOF_MAX_JSON_BYTES
        || parity
            .as_ref()
            .is_some_and(|value| value.len() > ACTIVATION_PROOF_MAX_JSON_BYTES)
    {
        return Err(migration_collision(
            "activated migration proof exceeds its bounded JSON size",
        ));
    }
    let parity = parity
        .as_deref()
        .ok_or_else(|| migration_collision("activated migration parity is missing"))?;
    let parity_value: serde_json::Value = serde_json::from_str(parity)
        .map_err(|_| migration_collision("activated migration parity is invalid"))?;
    let checkpoint_value: serde_json::Value = serde_json::from_str(&checkpoint)
        .map_err(|_| migration_collision("activated migration checkpoint is invalid"))?;
    if checkpoint_value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some("legacy_migration_checkpoint_v1")
        || checkpoint_value
            .get("substep")
            .and_then(serde_json::Value::as_str)
            != Some("authority_pointer")
        || checkpoint_value
            .get("read_only")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(migration_collision(
            "activated migration checkpoint is not at the pointer boundary",
        ));
    }
    let expected_inventory_sha256 = hex::encode(&stored_inventory_hash);
    if parity_value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some("legacy_migration_parity_v1")
        || parity_value
            .get("source_unchanged")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || parity_value
            .get("integrity")
            .and_then(serde_json::Value::as_str)
            != Some("ok")
        || parity_value
            .get("inventory_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(expected_inventory_sha256.as_str())
    {
        return Err(migration_collision(
            "activated migration parity proof is incomplete",
        ));
    }
    let source_count = parity_u64(
        &parity_value,
        "source_count",
        "activated migration source count is missing",
    )?;
    let raw_record_count = parity_value
        .get("raw_record_count")
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| migration_collision("activated migration raw count is invalid"))?;
    let (_, _, _, semantic_event_count, agent_semantic_stats) =
        semantic_parity_counts(&parity_value)?;
    if source_count != raw_record_count {
        return Err(migration_collision(
            "activated migration source/raw summary parity mismatch",
        ));
    }
    let source_inventory_hash: [u8; 32] = stored_inventory_hash
        .as_slice()
        .try_into()
        .map_err(|_| migration_collision("activated inventory hash has an invalid length"))?;
    let (
        generation_id,
        operation_kind,
        idempotency_key,
        payload_hash,
        state,
        event_commit_count,
        mutation_count,
        result_hash,
    ): (
        String,
        String,
        String,
        Vec<u8>,
        String,
        i64,
        i64,
        Option<Vec<u8>>,
    ) = connection.query_row(
        "SELECT generation_id, operation_kind, idempotency_key, payload_hash,
                    state, event_count, mutation_count, result_hash
             FROM logical_commits WHERE commit_id = ?1",
        params![commit_id],
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
    )?;
    let metadata_generation_id: String = connection.query_row(
        "SELECT generation_id FROM store_metadata WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    let expected_proof = activation_proof_digest(
        migration_id,
        &commit_id,
        &generation_id,
        migration_revision,
        mutation_count,
        &source_inventory_hash,
        &checkpoint,
        parity,
    );
    if operation_kind != "migration"
        || idempotency_key != format!("legacy-migration-{migration_id}")
        || payload_hash.as_slice() != source_inventory_hash.as_slice()
        || state != "sealed"
        || event_commit_count != 0
        || migration_revision < 0
        || mutation_count < 0
        || generation_id != metadata_generation_id
        || result_hash.as_deref() != Some(expected_proof.as_slice())
    {
        return Err(migration_collision(
            "activated migration proof does not match its sealed commit",
        ));
    }
    Ok(ActivatedMigrationProof {
        source_inventory_hash,
        source_count,
        raw_record_count,
        semantic_event_count,
        agent_semantic_stats,
        commit_id,
    })
}

pub(crate) fn verify_activated_migration_anchor(
    connection: &Connection,
    migration_id: &str,
) -> Result<(), rusqlite::Error> {
    load_activated_migration_proof(connection, migration_id).map(drop)
}

pub(crate) fn verify_activated_migration(
    connection: &Connection,
    migration_id: &str,
) -> Result<(), rusqlite::Error> {
    let ActivatedMigrationProof {
        source_inventory_hash,
        source_count,
        raw_record_count,
        semantic_event_count,
        agent_semantic_stats,
        commit_id,
    } = load_activated_migration_proof(connection, migration_id)?;
    let mut inventory_statement = connection.prepare(
        "SELECT source_ordinal, source_path, source_size, modified_ms,
                raw_sha256, record_count
         FROM legacy_source_inventory
         WHERE migration_id = ?1 ORDER BY source_ordinal ASC",
    )?;
    let inventory_rows = inventory_statement.query_map(params![migration_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Vec<u8>>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut saved_inventory = Vec::new();
    for (position, row) in inventory_rows.enumerate() {
        let (ordinal, relative_path, size, modified_ms, sha256, record_count) = row?;
        if ordinal != i64::try_from(position).unwrap_or(i64::MAX) || record_count != 1 {
            return Err(migration_collision(
                "activated migration inventory ordinal is not contiguous",
            ));
        }
        saved_inventory.push(SourceRecord {
            relative_path,
            size: size
                .parse()
                .map_err(|_| migration_collision("activated inventory size is invalid"))?,
            modified_ms: modified_ms
                .parse()
                .map_err(|_| migration_collision("activated inventory mtime is invalid"))?,
            sha256: sha256.try_into().map_err(|_| {
                migration_collision("activated inventory hash has an invalid length")
            })?,
        });
    }
    if u64::try_from(saved_inventory.len()).unwrap_or(u64::MAX) != source_count
        || inventory_hash(&saved_inventory) != source_inventory_hash
    {
        return Err(migration_collision(
            "activated migration immutable inventory parity mismatch",
        ));
    }
    let mut raw_statement = connection.prepare(
        "SELECT source_ordinal, source_path, source_size, modified_ms,
                record_count, raw, raw_sha256
         FROM legacy_raw_records
         WHERE migration_id = ?1 ORDER BY source_ordinal ASC",
    )?;
    let raw_rows = raw_statement.query_map(params![migration_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, Vec<u8>>(5)?,
            row.get::<_, Vec<u8>>(6)?,
        ))
    })?;
    let mut saved_raw_count = 0_u64;
    for (position, row) in raw_rows.enumerate() {
        let (ordinal, relative_path, size, modified_ms, record_count, raw, raw_sha256) = row?;
        let Some(expected) = saved_inventory.get(position) else {
            return Err(migration_collision(
                "activated migration contains an unexpected raw record",
            ));
        };
        let raw_sha256: [u8; 32] = raw_sha256
            .try_into()
            .map_err(|_| migration_collision("activated raw hash has an invalid length"))?;
        let raw_bytes_match = if expected.size > IMPORT_BYTE_LIMIT as u64 {
            if !raw.is_empty() {
                false
            } else {
                let mut chunks = connection.prepare(
                    "SELECT chunk_ordinal, source_offset, raw, raw_sha256
                     FROM legacy_raw_record_chunks
                     WHERE migration_id = ?1 AND source_ordinal = ?2
                     ORDER BY chunk_ordinal ASC",
                )?;
                let chunk_rows = chunks.query_map(params![migration_id, ordinal], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                })?;
                let mut chunk_count = 0_u64;
                let mut source_offset = 0_u64;
                let mut source_hasher = Sha256::new();
                let mut valid = true;
                for row in chunk_rows {
                    let (chunk_ordinal, saved_offset, chunk, chunk_sha256) = row?;
                    let calculated_chunk_sha256: [u8; 32] = Sha256::digest(&chunk).into();
                    if u64::try_from(chunk_ordinal).ok() != Some(chunk_count)
                        || saved_offset.parse::<u64>().ok() != Some(source_offset)
                        || chunk.is_empty()
                        || chunk.len() > RAW_CHUNK_LIMIT
                        || chunk_sha256.as_slice() != calculated_chunk_sha256.as_slice()
                    {
                        valid = false;
                        break;
                    }
                    source_hasher.update(&chunk);
                    source_offset = source_offset
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| migration_collision("activated raw offset overflow"))?;
                    chunk_count = chunk_count
                        .checked_add(1)
                        .ok_or_else(|| migration_collision("activated raw chunk overflow"))?;
                }
                let source_digest: [u8; 32] = source_hasher.finalize().into();
                valid
                    && chunk_count > 0
                    && source_offset == expected.size
                    && source_digest == expected.sha256
            }
        } else {
            u64::try_from(raw.len()).unwrap_or(u64::MAX) == expected.size
                && <[u8; 32]>::from(Sha256::digest(&raw)) == expected.sha256
        };
        if ordinal != i64::try_from(position).unwrap_or(i64::MAX)
            || relative_path != expected.relative_path
            || size.parse::<u64>().ok() != Some(expected.size)
            || modified_ms.parse::<u128>().ok() != Some(expected.modified_ms)
            || record_count != 1
            || raw_sha256 != expected.sha256
            || !raw_bytes_match
        {
            return Err(migration_collision(
                "activated migration raw record no longer matches its immutable inventory",
            ));
        }
        saved_raw_count = saved_raw_count
            .checked_add(1)
            .ok_or_else(|| migration_collision("activated raw count overflow"))?;
    }
    if source_count != raw_record_count || raw_record_count != saved_raw_count {
        return Err(migration_collision(
            "activated migration source/raw parity mismatch",
        ));
    }
    // Direct terminal rows and semantic events are immutable participants, so
    // they remain safe to validate after normal admission opens. Session and
    // message projections, queue/permission state, titles, and obligations are
    // intentionally not re-counted: ordinary post-cutover commands mutate or
    // resolve those rows.
    let migrated_terminal_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM terminal_records WHERE commit_id = ?1",
        params![commit_id],
        |row| row.get(0),
    )?;
    if u64::try_from(migrated_terminal_count).unwrap_or(u64::MAX)
        != agent_semantic_stats.terminal_count
    {
        return Err(migration_collision(
            "activated migration immutable terminal parity mismatch",
        ));
    }
    let migrated_stop_resolution_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM stop_resolutions WHERE commit_id = ?1",
        params![commit_id],
        |row| row.get(0),
    )?;
    if u64::try_from(migrated_stop_resolution_count).unwrap_or(u64::MAX)
        != agent_semantic_stats.stop_resolution_count
    {
        return Err(migration_collision(
            "activated migration immutable Stop resolution parity mismatch",
        ));
    }
    if stored_semantic_event_count(connection)? != semantic_event_count {
        return Err(migration_collision(
            "activated migration semantic event parity mismatch",
        ));
    }
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(migration_collision(
            "activated migration SQLite integrity check failed",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod bounded_tests {
    use super::*;
    use crate::domain::local_event::SessionProjectionRecord;
    use crate::usecase::agent_session::session::{
        AgentSessionProjectionCodec, CanonicalAgentSessionProjection,
    };

    pub(crate) const SEMANTIC_SESSION_ID: &str = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
    pub(crate) const CLOSED_SEMANTIC_SESSION_ID: &str = "b1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
    pub(crate) const ARCHIVED_SEMANTIC_SESSION_ID: &str = "c1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";

    fn decode_canonical_agent_session_projection(raw: &str) -> CanonicalAgentSessionProjection {
        let projection =
            crate::adaptor::gateway::agent_session::session_storage::decode_agent_session_projection_record_v1(
                raw,
            )
            .expect("decode stored agent-session projection");
        crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
            .decode(&SessionProjectionRecord::AgentSession(Box::new(projection)))
            .expect("decode semantic agent-session projection")
    }

    fn encode_canonical_agent_session_projection(
        projection: &CanonicalAgentSessionProjection,
    ) -> String {
        let SessionProjectionRecord::AgentSession(projection) =
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1
                .encode(projection)
                .expect("encode semantic agent-session projection")
        else {
            panic!("agent-session codec returned the wrong projection family");
        };
        crate::adaptor::gateway::agent_session::session_storage::encode_agent_session_projection_record_v1(
            &projection,
        )
        .expect("encode stored agent-session projection")
    }

    pub(crate) fn semantic_migration_fixture(root: &Path) {
        use crate::domain::agent_session::entities::{
            PermissionRequest, PermissionRequestBody, PermissionRequestStatus,
        };
        use crate::domain::agent_session::events::{
            SendDisposition, SessionLifecycleKind, StopResolution, TurnTokenUsage,
        };
        use crate::domain::agent_session::value_objects::JsonPayload;
        use crate::usecase::agent_session::event_log::{AgentSessionEvent, PromptInput};
        use crate::usecase::agent_session::session::{build_new_session_with_id, SessionMeta};

        let sessions_root = root.join("sessions");
        let session_dir = sessions_root.join(SEMANTIC_SESSION_ID);
        std::fs::create_dir_all(&session_dir).expect("legacy session directory");
        let mut session = build_new_session_with_id(
            SEMANTIC_SESSION_ID.to_string(),
            "/repo",
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Edit,
            Some("sonnet".to_string()),
            false,
            false,
            None,
        );
        session.created_at = 1_000.0;
        session.updated_at = 2_000.0;
        let meta = SessionMeta::from_session(&session);
        std::fs::write(
            session_dir.join("meta.json"),
            serde_json::to_vec_pretty(&meta).expect("legacy meta encode"),
        )
        .expect("legacy meta");
        for (session_id, state, provider_id, provider_generation, updated_at) in [
            (
                CLOSED_SEMANTIC_SESSION_ID,
                crate::usecase::agent_session::session::SessionState::Closed,
                "provider-closed",
                4,
                3_000.0,
            ),
            (
                ARCHIVED_SEMANTIC_SESSION_ID,
                crate::usecase::agent_session::session::SessionState::Archived,
                "provider-archived",
                7,
                4_000.0,
            ),
        ] {
            let session_dir = sessions_root.join(session_id);
            std::fs::create_dir_all(&session_dir).expect("legacy lifecycle session directory");
            let mut lifecycle_session = build_new_session_with_id(
                session_id.to_string(),
                "/repo",
                Some("claude".to_string()),
                crate::domain::agent_session::PermissionMode::Edit,
                Some("sonnet".to_string()),
                false,
                false,
                None,
            );
            lifecycle_session.state = state;
            lifecycle_session.created_at = 1_000.0;
            lifecycle_session.updated_at = updated_at;
            lifecycle_session.agent_session_id = Some(provider_id.to_string());
            let mut meta = SessionMeta::from_session(&lifecycle_session);
            meta.provider_session_generation = provider_generation;
            std::fs::write(
                session_dir.join("meta.json"),
                serde_json::to_vec_pretty(&meta).expect("legacy lifecycle meta encode"),
            )
            .expect("legacy lifecycle meta");
        }
        let events = vec![
            AgentSessionEvent::TurnStarted {
                turn_id: 1,
                message_id: "human-1".to_string(),
                assistant_message_id: Some("agent-1".to_string()),
                prompt: PromptInput::default(),
                at: 10.0,
            },
            AgentSessionEvent::TurnCompleted {
                turn_id: 1,
                exit_code: 0,
                stop_reason: None,
                token_usage: Some(TurnTokenUsage {
                    input_tokens: 30,
                    output_tokens: 12,
                }),
            },
            AgentSessionEvent::TurnStarted {
                turn_id: 2,
                message_id: "human-2".to_string(),
                assistant_message_id: Some("agent-2".to_string()),
                prompt: PromptInput::default(),
                at: 20.0,
            },
            AgentSessionEvent::PermissionRequested {
                turn_id: 2,
                tool_use_id: Some("tool-2".to_string()),
                request: PermissionRequest {
                    id: "permission-2".to_string(),
                    tool_use_id: Some("tool-2".to_string()),
                    parent_tool_use_id: None,
                    tool_name: "Bash".to_string(),
                    body: PermissionRequestBody::ToolApproval {
                        input: JsonPayload::new_unchecked(
                            "{\"command\":\"cargo test\"}".to_string(),
                        ),
                    },
                    title: None,
                    display_name: None,
                    description: None,
                    decision_reason: None,
                    status: PermissionRequestStatus::Pending,
                },
            },
            AgentSessionEvent::SendOperationAccepted {
                operation_id: "send-queued-1".to_string(),
                disposition: SendDisposition::Queued {
                    queue_item_id: "queue-1".to_string(),
                },
                human_message_id: Some("human-3".to_string()),
                prompt: Some(PromptInput::default()),
                reserved_turn_id: Some("3".to_string()),
                at: 21.0,
            },
            AgentSessionEvent::QueuePaused { at: 22.0 },
            AgentSessionEvent::StopOperationAccepted {
                operation_id: "stop-2".to_string(),
                target_turn_id: 2,
                at: 23.0,
            },
            AgentSessionEvent::StopResolutionRecorded {
                operation_id: "stop-2".to_string(),
                turn_id: 2,
                resolution: StopResolution::Superseded,
                at: 24.0,
            },
            AgentSessionEvent::SessionLifecycleOperationAccepted {
                operation_id: "lifecycle-2".to_string(),
                kind: SessionLifecycleKind::Archive,
                at: 25.0,
            },
        ];
        std::fs::write(
            session_dir.join("events.json"),
            crate::adaptor::gateway::agent_session::session_storage::encode_agent_session_events_v1(
                &events,
                true,
            )
            .expect("legacy events encode"),
        )
        .expect("legacy events");
        std::fs::write(
            root.join("session_titles.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                SEMANTIC_SESSION_ID: "Migrated title",
                CLOSED_SEMANTIC_SESSION_ID: "Closed migrated title",
                ARCHIVED_SEMANTIC_SESSION_ID: "Archived migrated title",
            }))
            .expect("legacy titles encode"),
        )
        .expect("legacy titles");
    }

    fn migration_connection(path: &Path) -> Connection {
        let connection = Connection::open(path).expect("migration database");
        super::super::schema::apply_schema(&connection).expect("migration schema");
        connection
            .execute(
                "INSERT OR IGNORE INTO store_metadata (
                    id, schema_version, store_id, generation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key, boot_id,
                    next_global_sequence, health, current_shutdown_plan_id,
                    current_shutdown_epoch, shutdown_pointer_revision
                 ) VALUES (1, 1, 'store-test', 'generation-test', 1,
                           ?1, ?2, 'boot-test', 1, 'recovering', NULL, NULL, 0)",
                params![[1_u8; 32].as_slice(), [2_u8; 32].as_slice()],
            )
            .expect("migration metadata");
        connection
    }

    fn assert_semantic_migration_projection(connection: &Connection) {
        use crate::usecase::agent_session::session::SessionState;

        let projection: String = connection
            .query_row(
                "SELECT projection FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("migrated session projection");
        let projection = decode_canonical_agent_session_projection(&projection);
        assert_eq!(projection.title.as_deref(), Some("Migrated title"));
        assert_eq!(projection.meta.state, SessionState::Error);
        assert_eq!(projection.meta.last_turn_id, Some(2));
        assert_eq!(projection.queue_paused_at, Some(22.0));
        assert_eq!(projection.pending_send_queue.len(), 1);
        assert_eq!(projection.pending_send_queue[0].queue_item_id, "queue-1");
        let usage = projection.latest_token_usage.expect("latest token usage");
        assert_eq!((usage.input_tokens, usage.output_tokens), (30, 12));
        for (session_id, expected_state, provider_id, provider_generation) in [
            (
                CLOSED_SEMANTIC_SESSION_ID,
                SessionState::Closed,
                "provider-closed",
                4,
            ),
            (
                ARCHIVED_SEMANTIC_SESSION_ID,
                SessionState::Archived,
                "provider-archived",
                7,
            ),
        ] {
            let projection: String = connection
                .query_row(
                    "SELECT projection FROM session_projection WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .expect("migrated lifecycle projection");
            let projection = decode_canonical_agent_session_projection(&projection);
            assert_eq!(projection.meta.state, expected_state);
            assert_eq!(
                projection.meta.agent_session_id.as_deref(),
                Some(provider_id)
            );
            assert_eq!(
                projection.meta.provider_session_generation,
                provider_generation
            );
        }

        let terminal_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM terminal_records
                 WHERE session_id = ?1 AND turn_id = '1'",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("terminal count");
        assert_eq!(terminal_count, 1);
        let pending: (i64, i64) = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM obligations
                     WHERE obligation_id LIKE 'legacy-%' AND pending = 1),
                    (SELECT COUNT(*) FROM pending_obligations
                     WHERE obligation_id LIKE 'legacy-%')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("pending parity");
        assert_eq!(pending, (6, 6));
        let partitions: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pending_obligations
                 WHERE obligation_id LIKE 'legacy-%'
                   AND owner = ?1 AND partition = 'owner'",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("owner partition parity");
        assert_eq!(partitions, 6);
        let operation_read_models: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM operation_records
                 WHERE operation_id IN ('send-queued-1', 'stop-2', 'lifecycle-2')",
                [],
                |row| row.get(0),
            )
            .expect("unprovable operation read models");
        assert_eq!(operation_read_models, 0);
        let operation_quarantines: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM obligations
                 WHERE obligation_id LIKE 'legacy-%'
                   AND record LIKE '%\"kind\":\"operation_binding\"%'",
                [],
                |row| row.get(0),
            )
            .expect("operation quarantine count");
        assert_eq!(operation_quarantines, 3);
        let stop_resolution: (String, String) = connection
            .query_row(
                "SELECT resolution, detail FROM stop_resolutions
                 WHERE stop_operation_id = 'stop-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("migrated Stop resolution");
        assert_eq!(stop_resolution.0, "superseded");
        assert!(stop_resolution
            .1
            .contains("\"schema\":\"legacy_stop_resolution_v1\""));

        let query_context = crate::adaptor::gateway::local_event_store::reader::QueryContext {
            registry: std::sync::Arc::new(EventCodecRegistry::new()),
            cursor_key: vec![7_u8; 32],
            boot_id: "migration-test-boot".to_string(),
            clock: std::sync::Arc::new(
                crate::adaptor::gateway::local_event_store::clock::FakeStoreClock::at(1_000),
            ),
        };
        let stop_lookup = crate::adaptor::gateway::local_event_store::reader::run_query(
            connection,
            &query_context,
            &crate::domain::local_event::LocalEventQuery::StopResolutionByOperation {
                stop_operation_id: "stop-2".to_string(),
            },
        )
        .expect("public Stop resolution lookup");
        let crate::domain::local_event::LocalEventQueryResult::StopResolutionByOperation(Some(
            stop_lookup,
        )) = stop_lookup
        else {
            panic!("migrated Stop resolution must have a direct public lookup");
        };
        assert_eq!(
            stop_lookup.resolution,
            crate::domain::local_event::StopResolutionKind::Superseded
        );
        let operation_lookup = crate::adaptor::gateway::local_event_store::reader::run_query(
            connection,
            &query_context,
            &crate::domain::local_event::LocalEventQuery::OperationByIdentity {
                kind: crate::domain::local_event::OperationKind::Stop,
                operation_id: "stop-2".to_string(),
            },
        )
        .expect("public unprovable operation lookup");
        assert!(matches!(
            operation_lookup,
            crate::domain::local_event::LocalEventQueryResult::OperationByIdentity(None)
        ));
        let quarantine_id: String = connection
            .query_row(
                "SELECT obligation_id FROM obligations
                 WHERE record LIKE '%\"operation_id\":\"stop-2\"%'
                   AND record LIKE '%\"kind\":\"operation_binding\"%'",
                [],
                |row| row.get(0),
            )
            .expect("Stop operation quarantine identity");
        let quarantine_lookup = crate::adaptor::gateway::local_event_store::reader::run_query(
            connection,
            &query_context,
            &crate::domain::local_event::LocalEventQuery::ObligationByIdentity {
                obligation_id: quarantine_id,
            },
        )
        .expect("public operation quarantine lookup");
        let crate::domain::local_event::LocalEventQueryResult::ObligationByIdentity(Some(
            quarantine,
        )) = quarantine_lookup
        else {
            panic!("unprovable legacy operation must remain supervised pending work");
        };
        assert_eq!(
            quarantine
                .pending
                .expect("operation quarantine pending index")
                .owner,
            SEMANTIC_SESSION_ID
        );
    }

    #[test]
    fn semantic_migration_replays_checkpoint_and_reopens_without_losing_state() {
        let root = tempfile::TempDir::new().expect("semantic migration app data");
        semantic_migration_fixture(root.path());
        let database_path = root.path().join("staging.sqlite3");
        let mut connection = migration_connection(&database_path);

        let first = import_legacy(
            &mut connection,
            root.path(),
            "migration-semantic",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("first semantic import");
        assert_semantic_migration_projection(&connection);
        let first_counts: (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM events),
                    (SELECT COUNT(*) FROM terminal_records),
                    (SELECT COUNT(*) FROM obligations),
                    (SELECT COUNT(*) FROM stop_resolutions)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("first semantic counts");

        let replay = import_legacy(
            &mut connection,
            root.path(),
            "migration-semantic",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("checkpoint replay");
        assert_eq!(replay, first);
        let replay_counts: (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM events),
                    (SELECT COUNT(*) FROM terminal_records),
                    (SELECT COUNT(*) FROM obligations),
                    (SELECT COUNT(*) FROM stop_resolutions)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("replayed semantic counts");
        assert_eq!(replay_counts, first_counts);
        assert_semantic_migration_projection(&connection);
        let import_commit: (String, Option<Vec<u8>>) = connection
            .query_row(
                "SELECT state, result_hash FROM logical_commits
                 WHERE commit_id = 'migration-migration-semantic'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("unsealed migration import boundary");
        assert_eq!(import_commit, ("preparing".to_string(), None));

        mark_activating(
            &mut connection,
            "migration-semantic",
            "boot-activated",
            first,
        )
        .expect("mark semantic migration activating");
        let activation_commit: (String, Vec<u8>, Vec<u8>) = connection
            .query_row(
                "SELECT state, payload_hash, result_hash FROM logical_commits
                 WHERE commit_id = 'migration-migration-semantic'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("sealed migration activation proof");
        assert_eq!(activation_commit.0, "sealed");
        assert_eq!(activation_commit.1.as_slice(), first.as_slice());
        assert_eq!(activation_commit.2.len(), 32);
        assert_eq!(
            activated_migration_inventory_hash(&connection, "migration-semantic")
                .expect("activation resume locator"),
            Some(first)
        );
        verify_activated_migration(&connection, "migration-semantic")
            .expect("activated semantic parity");
        verify_activated_migration_anchor(&connection, "migration-semantic")
            .expect("bounded activation anchor");

        {
            let transaction = connection.transaction().expect("proof corruption fixture");
            transaction
                .execute(
                    "UPDATE logical_commits SET result_hash = zeroblob(32)
                     WHERE commit_id = 'migration-migration-semantic'",
                    [],
                )
                .expect("corrupt sealed proof");
            assert!(
                verify_activated_migration_anchor(&transaction, "migration-semantic").is_err(),
                "bounded reader proof must reject a corrupt sealed result hash"
            );
            transaction.rollback().expect("restore sealed proof");
        }
        verify_activated_migration_anchor(&connection, "migration-semantic")
            .expect("restored bounded activation anchor");

        {
            let transaction = connection.transaction().expect("raw scan boundary fixture");
            transaction
                .execute(
                    "DELETE FROM legacy_raw_records
                     WHERE migration_id = 'migration-semantic'",
                    [],
                )
                .expect("remove raw migration records");
            verify_activated_migration_anchor(&transaction, "migration-semantic")
                .expect("bounded proof must not rescan raw legacy bytes");
            assert!(
                verify_activated_migration(&transaction, "migration-semantic").is_err(),
                "exclusive-writer verification must retain the full raw parity scan"
            );
            transaction
                .rollback()
                .expect("restore raw migration records");
        }
        verify_activated_migration(&connection, "migration-semantic")
            .expect("full verification remains intact after proof tests");

        // Normal post-cutover work is allowed to evolve every mutable value
        // that was counted at the activation boundary. Reopen must validate
        // the immutable activation proof, not demand the original UI state.
        use crate::usecase::agent_session::session::SessionState;
        let current: String = connection
            .query_row(
                "SELECT projection FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("current semantic projection");
        let mut evolved = decode_canonical_agent_session_projection(&current);
        evolved.title = None;
        evolved.pending_send_queue.clear();
        evolved.reducer_events.clear();
        evolved.queue_paused_at = None;
        evolved.meta.state = SessionState::Closed;
        let evolved = encode_canonical_agent_session_projection(&evolved);
        connection
            .execute(
                "UPDATE session_projection
                 SET projection = ?2, revision = revision + 1
                 WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID, evolved],
            )
            .expect("evolve migrated projection");
        connection
            .execute(
                "UPDATE obligations SET pending = 0, revision = revision + 1
                 WHERE obligation_id LIKE 'legacy-%'",
                [],
            )
            .expect("resolve migrated obligations");
        connection
            .execute(
                "DELETE FROM pending_obligations WHERE obligation_id LIKE 'legacy-%'",
                [],
            )
            .expect("remove resolved pending indexes");
        verify_activated_migration(&connection, "migration-semantic")
            .expect("post-cutover mutable state does not invalidate proof");
        drop(connection);

        let reopened = Connection::open(database_path).expect("reopen semantic database");
        verify_activated_migration(&reopened, "migration-semantic")
            .expect("reopened semantic parity");
        let mutable_counts: (i64, i64, i64) = reopened
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM obligations
                     WHERE obligation_id LIKE 'legacy-%' AND pending = 1),
                    (SELECT COUNT(*) FROM pending_obligations
                     WHERE obligation_id LIKE 'legacy-%'),
                    (SELECT COUNT(*) FROM session_projection
                     WHERE session_id = ?1 AND revision = 1)",
                params![SEMANTIC_SESSION_ID],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("evolved mutable state after reopen");
        assert_eq!(mutable_counts, (0, 0, 1));
    }

    #[test]
    fn source_record_count_boundaries_import_exactly_once() {
        for source_count in [0_usize, 1, 255, 256, 257] {
            let root = tempfile::TempDir::new().expect("migration app data");
            if source_count > 0 {
                let sessions = root.path().join("sessions");
                std::fs::create_dir_all(&sessions).expect("legacy sessions directory");
                for ordinal in 0..source_count {
                    std::fs::write(sessions.join(format!("source-{ordinal:03}.json")), b"{}")
                        .expect("legacy source fixture");
                }
            }
            let mut connection = migration_connection(&root.path().join("staging.sqlite3"));
            let migration_id = format!("source-count-{source_count}");

            let inventory_hash = import_legacy(
                &mut connection,
                root.path(),
                &migration_id,
                "generation-test",
                1_000,
                |_| {},
            )
            .unwrap_or_else(|error| panic!("source count {source_count} failed: {error}"));

            let counts: (i64, i64) = connection
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM legacy_source_inventory
                         WHERE migration_id = ?1),
                        (SELECT COUNT(*) FROM legacy_raw_records
                         WHERE migration_id = ?1)",
                    params![migration_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("source boundary counts");
            assert_eq!(counts, (source_count as i64, source_count as i64));
            let parity: String = connection
                .query_row(
                    "SELECT parity FROM local_store_migrations WHERE migration_id = ?1",
                    params![migration_id],
                    |row| row.get(0),
                )
                .expect("source boundary parity");
            let parity: serde_json::Value =
                serde_json::from_str(&parity).expect("source boundary parity JSON");
            assert_eq!(parity["source_count"].as_u64(), Some(source_count as u64));
            assert_eq!(
                parity["raw_record_count"].as_i64(),
                Some(source_count as i64)
            );
            verify_source_unchanged(root.path(), inventory_hash, || {})
                .expect("fixed source remains verifiable");
        }
    }

    #[test]
    fn inventory_retains_metadata_and_enforces_exact_sixteen_mibibyte_boundary() {
        let root = tempfile::TempDir::new().expect("migration app data");
        let sessions = root.path().join("sessions");
        std::fs::create_dir_all(&sessions).expect("legacy sessions directory");
        let exact = sessions.join("exact.bin");
        let exact_file = std::fs::File::create(&exact).expect("exact fixture");
        exact_file
            .set_len(IMPORT_BYTE_LIMIT as u64)
            .expect("size exact fixture");
        let oversized = sessions.join("oversized.json");
        let file = std::fs::File::create(&oversized).expect("oversized fixture");
        file.set_len((IMPORT_BYTE_LIMIT + 1) as u64)
            .expect("size oversized fixture");

        let records = inventory(root.path()).expect("metadata inventory");
        assert_eq!(records.len(), 2);
        assert!(std::mem::size_of::<SourceRecord>() < 256);
        let exact = records
            .iter()
            .find(|record| record.relative_path.ends_with("exact.bin"))
            .expect("exact metadata");
        assert_eq!(exact.size, IMPORT_BYTE_LIMIT as u64);
        assert_eq!(
            read_inventoried_source(root.path(), exact)
                .expect("exact boundary is readable")
                .len(),
            IMPORT_BYTE_LIMIT
        );
        let oversized = records
            .iter()
            .find(|record| record.relative_path.ends_with("oversized.json"))
            .expect("oversized metadata");
        assert_eq!(oversized.size, (IMPORT_BYTE_LIMIT + 1) as u64);
        assert!(read_inventoried_source(root.path(), oversized).is_err());
    }

    #[test]
    fn fixed_inventory_rejects_source_drift_before_import() {
        let root = tempfile::TempDir::new().expect("migration app data");
        let sessions = root.path().join("sessions");
        std::fs::create_dir_all(&sessions).expect("legacy sessions directory");
        let source = sessions.join("drift.bin");
        std::fs::write(&source, b"before").expect("legacy source fixture");
        let records = inventory(root.path()).expect("fixed inventory");
        let expected_hash = inventory_hash(&records);

        std::fs::write(&source, b"after-and-different-length").expect("drift source");

        let read_error = read_inventoried_source(root.path(), &records[0]).unwrap_err();
        assert!(read_error.to_string().contains("changed"));
        let verify_error = verify_source_unchanged(root.path(), expected_hash, || {}).unwrap_err();
        assert!(verify_error.to_string().contains("changed"));
    }

    #[test]
    fn semantic_identity_collision_blocks_migration() {
        let root = tempfile::TempDir::new().expect("migration database");
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));
        connection
            .execute(
                "INSERT INTO logical_commits (
                    commit_id, generation_id, operation_kind, idempotency_key, payload_hash,
                    state, first_global_sequence, last_global_sequence, event_count,
                    mutation_count, stream_heads_json, result_hash, committed_at_ms
                 ) VALUES ('commit-a', 'generation-test', 'migration', 'collision', ?1,
                           'sealed', NULL, NULL, 0, 1, '[]', ?1, 1)",
                params![[0_u8; 32].as_slice()],
            )
            .expect("collision logical commit");
        let transaction = connection.transaction().expect("collision transaction");
        insert_session_projection(
            &transaction,
            SEMANTIC_SESSION_ID,
            "projection-a",
            "commit-a",
        )
        .expect("first semantic identity");

        let error = insert_session_projection(
            &transaction,
            SEMANTIC_SESSION_ID,
            "projection-b",
            "commit-a",
        )
        .unwrap_err();

        assert!(error.to_string().contains("identity collision"));
    }

    #[test]
    fn unknown_additive_event_bytes_are_preserved_during_semantic_import() {
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        let event_path = root
            .path()
            .join("sessions")
            .join(SEMANTIC_SESSION_ID)
            .join("events.json");
        let mut events: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&event_path).expect("legacy events"))
                .expect("legacy event JSON");
        events[0]["future_additive"] = serde_json::json!({ "nested": true });
        let additive_raw = serde_json::to_vec_pretty(&events).expect("additive event JSON");
        std::fs::write(&event_path, &additive_raw).expect("additive legacy event");
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        import_legacy(
            &mut connection,
            root.path(),
            "unknown-additive",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("additive event remains semantically readable");

        let preserved: Vec<u8> = connection
            .query_row(
                "SELECT raw FROM legacy_raw_records
                 WHERE migration_id = 'unknown-additive' AND source_path = ?1",
                params![format!("sessions/{SEMANTIC_SESSION_ID}/events.json")],
                |row| row.get(0),
            )
            .expect("preserved additive source");
        assert_eq!(preserved, additive_raw);
    }

    #[test]
    fn unknown_required_event_semantics_block_migration() {
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        std::fs::write(
            root.path()
                .join("sessions")
                .join(SEMANTIC_SESSION_ID)
                .join("events.json"),
            br#"[{"type":"future_required_event","at":1.0}]"#,
        )
        .expect("required unknown event");
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        let error = import_legacy(
            &mut connection,
            root.path(),
            "unknown-required",
            "generation-test",
            1_000,
            |_| {},
        )
        .unwrap_err();

        assert!(error.to_string().contains("incompatible"));
        let pointer_ready: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM local_store_migrations
                 WHERE migration_id = 'unknown-required' AND phase = 'verifying'",
                [],
                |row| row.get(0),
            )
            .expect("blocked migration phase");
        assert_eq!(pointer_ready, 0);
    }

    #[test]
    fn chunk_selector_never_widens_the_sixteen_mibibyte_limit_for_first_record() {
        let records = [
            SourceRecord {
                relative_path: "sessions/exact.json".to_string(),
                size: IMPORT_BYTE_LIMIT as u64,
                modified_ms: 0,
                sha256: [0; 32],
            },
            SourceRecord {
                relative_path: "sessions/next.json".to_string(),
                size: 1,
                modified_ms: 0,
                sha256: [0; 32],
            },
        ];
        let mut end = 0;
        let mut bytes = 0usize;
        while end < records.len()
            && end < IMPORT_RECORD_LIMIT
            && bytes.saturating_add(records[end].size as usize) <= IMPORT_BYTE_LIMIT
        {
            bytes += records[end].size as usize;
            end += 1;
        }
        assert_eq!(end, 1);
        assert_eq!(bytes, IMPORT_BYTE_LIMIT);
    }

    fn write_legacy_message(path: &Path, ordinal: u64) {
        let value = serde_json::json!({
            "id": format!("message-{ordinal}"),
            "role": "human",
            "content": format!("content-{ordinal}"),
            "streamingFinalSeq": 0,
            "timestamp": ordinal as f64,
            "parts": [{ "type": "text", "content": format!("part-{ordinal}") }],
        });
        std::fs::write(
            path,
            serde_json::to_vec(&value).expect("legacy message JSON"),
        )
        .expect("legacy message fixture");
    }

    #[test]
    fn legacy_message_pages_follow_numeric_semantic_ordinals_not_path_or_rowid_order() {
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        let message_dir = root
            .path()
            .join("sessions")
            .join(SEMANTIC_SESSION_ID)
            .join("messages");
        std::fs::create_dir_all(&message_dir).expect("legacy message directory");
        for ordinal in 1..=12 {
            write_legacy_message(&message_dir.join(format!("{ordinal}.json")), ordinal);
        }
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        import_legacy(
            &mut connection,
            root.path(),
            "numeric-message-order",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("numeric legacy message import");

        let query_context = crate::adaptor::gateway::local_event_store::reader::QueryContext {
            registry: std::sync::Arc::new(EventCodecRegistry::new()),
            cursor_key: vec![7_u8; 32],
            boot_id: "numeric-message-order-boot".to_string(),
            clock: std::sync::Arc::new(
                crate::adaptor::gateway::local_event_store::clock::FakeStoreClock::at(1_000),
            ),
        };
        let mut before_position = None;
        let mut observed = Vec::new();
        let mut observed_pages = Vec::new();
        loop {
            let page = crate::adaptor::gateway::local_event_store::reader::run_query(
                &connection,
                &query_context,
                &crate::domain::local_event::LocalEventQuery::MessageProjectionPage {
                    session_id: SEMANTIC_SESSION_ID.to_string(),
                    before_position,
                    limit: 5,
                },
            )
            .expect("legacy message page");
            let crate::domain::local_event::LocalEventQueryResult::MessageProjectionPage(page) =
                page
            else {
                panic!("message page query returned the wrong shape");
            };
            let page_ids = page
                .entries
                .iter()
                .map(|entry| entry.message.message_id.clone())
                .collect::<Vec<_>>();
            let page_positions = page
                .entries
                .iter()
                .map(|entry| entry.position)
                .collect::<Vec<_>>();
            let next_before_position = page.next_before_position;
            assert_eq!(page.total_count, 12);
            observed.splice(0..0, page_ids.iter().cloned());
            observed_pages.push((page_ids, page_positions, next_before_position));
            let Some(next) = next_before_position else {
                break;
            };
            before_position = Some(next);
        }
        assert_eq!(
            observed_pages,
            vec![
                (
                    (8..=12)
                        .map(|ordinal| format!("message-{ordinal}"))
                        .collect::<Vec<_>>(),
                    (8..=12).collect::<Vec<_>>(),
                    Some(8),
                ),
                (
                    (3..=7)
                        .map(|ordinal| format!("message-{ordinal}"))
                        .collect::<Vec<_>>(),
                    (3..=7).collect::<Vec<_>>(),
                    Some(3),
                ),
                (
                    (1..=2)
                        .map(|ordinal| format!("message-{ordinal}"))
                        .collect::<Vec<_>>(),
                    (1..=2).collect::<Vec<_>>(),
                    None,
                ),
            ]
        );
        assert_eq!(
            observed,
            (1..=12)
                .map(|ordinal| format!("message-{ordinal}"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn legacy_message_sequence_rejects_invalid_duplicate_and_missing_ordinals() {
        for (case, names) in [
            ("invalid", vec!["not-an-ordinal.json"]),
            ("duplicate", vec!["1.json", "01.json"]),
            ("missing", vec!["1.json", "3.json"]),
        ] {
            let root = tempfile::TempDir::new().expect("migration app data");
            semantic_migration_fixture(root.path());
            let message_dir = root
                .path()
                .join("sessions")
                .join(SEMANTIC_SESSION_ID)
                .join("messages");
            std::fs::create_dir_all(&message_dir).expect("legacy message directory");
            for (index, name) in names.iter().enumerate() {
                write_legacy_message(&message_dir.join(name), (index + 1) as u64);
            }
            let mut connection = migration_connection(&root.path().join("staging.sqlite3"));
            let error = import_legacy(
                &mut connection,
                root.path(),
                &format!("message-sequence-{case}"),
                "generation-test",
                1_000,
                |_| {},
            )
            .expect_err("invalid semantic message sequence must fail closed");
            assert!(
                error.to_string().contains("message ordinal"),
                "unexpected {case} failure: {error}"
            );
        }
    }

    fn add_private_context_semantics(root: &Path) {
        let session_dir = root.join("sessions").join(SEMANTIC_SESSION_ID);
        let meta_path = session_dir.join("meta.json");
        let mut meta: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&meta_path).expect("legacy meta fixture"))
                .expect("legacy meta JSON");
        meta["contextEpoch"] = serde_json::json!({
            "epochId": 7,
            "backendId": "claude",
            "modelId": "sonnet",
            "worktreePath": "/repo",
            "sourceRevisions": [{
                "kind": "repo_summary",
                "revision": 2,
                "fingerprint": "repo-fingerprint"
            }]
        });
        std::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&meta).expect("legacy meta encode"),
        )
        .expect("legacy meta with context epoch");
        std::fs::write(
            session_dir.join("private_context.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "workflowInstruction": "legacy workflow instruction",
                "workflowInstructions": ["current workflow instruction"],
                "contextEpochPayloads": [{
                    "kind": "repo_summary",
                    "fingerprint": "repo-fingerprint",
                    "payload": "cached repo summary"
                }],
                "agentReadPaths": ["/repo/src/lib.rs"]
            }))
            .expect("private context encode"),
        )
        .expect("private context fixture");
    }

    #[test]
    fn private_context_is_semantically_hydrated_into_the_sqlite_projection() {
        use crate::domain::agent_session::ContextSourceKind;
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        add_private_context_semantics(root.path());
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        let inventory_hash = import_legacy(
            &mut connection,
            root.path(),
            "private-context",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("private context migration");

        let projection: String = connection
            .query_row(
                "SELECT projection FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("migrated session projection");
        let projection = decode_canonical_agent_session_projection(&projection);
        assert_eq!(
            projection.meta.workflow_instructions,
            vec![
                "current workflow instruction".to_string(),
                "legacy workflow instruction".to_string(),
            ]
        );
        assert_eq!(
            projection.meta.agent_read_paths,
            Some(vec![std::path::PathBuf::from("/repo/src/lib.rs")])
        );
        assert_eq!(
            projection
                .meta
                .context_epoch
                .as_ref()
                .and_then(|epoch| epoch.payload_for(ContextSourceKind::RepoSummary)),
            Some("cached repo summary")
        );

        let parity: String = connection
            .query_row(
                "SELECT parity FROM local_store_migrations WHERE migration_id = 'private-context'",
                [],
                |row| row.get(0),
            )
            .expect("private-context parity");
        let parity: serde_json::Value =
            serde_json::from_str(&parity).expect("private-context parity JSON");
        assert_eq!(
            parity["semantic_workflow_instruction_count"].as_u64(),
            Some(2)
        );
        assert_eq!(
            parity["semantic_context_epoch_payload_count"].as_u64(),
            Some(1)
        );
        for key in [
            "semantic_workflow_instruction_sha256",
            "semantic_context_epoch_payload_sha256",
        ] {
            let digest = parity[key].as_str().expect("semantic parity digest");
            assert_eq!(digest.len(), 64);
            assert!(
                digest.bytes().any(|byte| byte != b'0'),
                "non-empty private context must not attest a zero digest"
            );
        }
        mark_activating(
            &mut connection,
            "private-context",
            "private-context-activation-boot",
            inventory_hash,
        )
        .expect("nonzero private-context parity must activate");
    }

    #[test]
    fn activation_rejects_semantic_projection_count_or_hash_tampering() {
        for tamper in [
            "workflow-instruction-hash",
            "context-payload-count",
            "agent-read-path-hash",
            "owner-relation-count",
        ] {
            let root = tempfile::TempDir::new().expect("migration app data");
            semantic_migration_fixture(root.path());
            add_private_context_semantics(root.path());
            let migration_id = format!("private-context-tamper-{tamper}");
            let mut connection = migration_connection(&root.path().join("staging.sqlite3"));
            let inventory_hash = import_legacy(
                &mut connection,
                root.path(),
                &migration_id,
                "generation-test",
                1_000,
                |_| {},
            )
            .expect("private context migration");

            let stored: String = connection
                .query_row(
                    "SELECT projection FROM session_projection WHERE session_id = ?1",
                    params![SEMANTIC_SESSION_ID],
                    |row| row.get(0),
                )
                .expect("migrated session projection");
            let mut projection = decode_canonical_agent_session_projection(&stored);
            match tamper {
                "workflow-instruction-hash" => {
                    projection.meta.workflow_instructions[0] =
                        "tampered workflow instruction".to_string();
                }
                "context-payload-count" => {
                    projection
                        .meta
                        .context_epoch
                        .as_mut()
                        .expect("context epoch")
                        .source_revisions[0]
                        .payload = None;
                }
                "agent-read-path-hash" => {
                    projection
                        .meta
                        .agent_read_paths
                        .as_mut()
                        .expect("agent read paths")[0] =
                        std::path::PathBuf::from("/repo/src/tampered.rs");
                }
                "owner-relation-count" => {
                    projection.meta.workflow_node_session = true;
                }
                _ => unreachable!(),
            }
            let tampered = encode_canonical_agent_session_projection(&projection);
            connection
                .execute(
                    "UPDATE session_projection SET projection = ?2 WHERE session_id = ?1",
                    params![SEMANTIC_SESSION_ID, tampered],
                )
                .expect("tamper staged projection");

            let error = mark_activating(
                &mut connection,
                &migration_id,
                "tamper-activation-boot",
                inventory_hash,
            )
            .expect_err("semantic projection tampering must block activation");
            assert!(
                error
                    .to_string()
                    .contains("semantic projection fields changed before activation"),
                "unexpected {tamper} failure: {error}"
            );
            let (phase, commit_state): (String, String) = connection
                .query_row(
                    "SELECT migration.phase, logical_commit.state
                     FROM local_store_migrations AS migration
                     JOIN logical_commits AS logical_commit
                       ON logical_commit.commit_id = migration.commit_id
                     WHERE migration.migration_id = ?1",
                    params![migration_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("blocked activation state");
            assert_eq!(
                (phase.as_str(), commit_state.as_str()),
                ("verifying", "preparing")
            );
        }
    }

    fn enlarge_semantic_event_source_past_batch_limit(root: &Path) -> u64 {
        let event_path = root
            .join("sessions")
            .join(SEMANTIC_SESSION_ID)
            .join("events.json");
        let mut events: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&event_path).expect("legacy event fixture"))
                .expect("legacy events JSON");
        let padding = "x".repeat(2 * 1024 * 1024);
        for event in events
            .as_array_mut()
            .expect("legacy fixture is an event array")
        {
            event["futureAdditive"] = serde_json::Value::String(padding.clone());
        }
        let oversized = serde_json::to_vec(&events).expect("oversized event source");
        assert!(oversized.len() > IMPORT_BYTE_LIMIT);
        std::fs::write(&event_path, oversized).expect("oversized valid legacy source");
        std::fs::metadata(event_path)
            .expect("oversized event metadata")
            .len()
    }

    fn replace_with_many_large_semantic_event_records(root: &Path, turn_count: u64) -> u64 {
        use crate::domain::agent_session::events::TurnTokenUsage;
        use crate::usecase::agent_session::event_log::{AgentSessionEvent, PromptInput};

        let event_path = root
            .join("sessions")
            .join(SEMANTIC_SESSION_ID)
            .join("events.json");
        let prompt_padding = "semantic-input-".repeat(1_600);
        let mut events = Vec::with_capacity(
            usize::try_from(turn_count.saturating_mul(2)).expect("bounded fixture count"),
        );
        for turn_id in 1..=turn_count {
            events.push(AgentSessionEvent::TurnStarted {
                turn_id,
                message_id: format!("human-{turn_id}"),
                assistant_message_id: Some(format!("agent-{turn_id}")),
                prompt: PromptInput {
                    content: format!("{turn_id}:{prompt_padding}"),
                    mentions: Vec::new(),
                    attachment_refs: Vec::new(),
                    parts: Vec::new(),
                },
                at: turn_id as f64,
            });
            events.push(AgentSessionEvent::TurnCompleted {
                turn_id,
                exit_code: 0,
                stop_reason: None,
                token_usage: Some(TurnTokenUsage {
                    input_tokens: turn_id,
                    output_tokens: 1,
                }),
            });
        }
        let encoded = crate::adaptor::gateway::agent_session::session_storage::encode_agent_session_events_v1(
            &events,
            true,
        )
        .expect("many semantic records encode");
        assert!(encoded.len() > IMPORT_BYTE_LIMIT);
        std::fs::write(&event_path, encoded).expect("many semantic records source");
        std::fs::metadata(event_path)
            .expect("many semantic records metadata")
            .len()
    }

    fn write_large_legacy_message_source(root: &Path) -> std::path::PathBuf {
        let message_dir = root
            .join("sessions")
            .join(SEMANTIC_SESSION_ID)
            .join("messages");
        std::fs::create_dir_all(&message_dir).expect("legacy message directory");
        let path = message_dir.join("1.json");
        let value = serde_json::json!({
            "id": "large-message-1",
            "role": "human",
            "content": "message-content-".repeat((IMPORT_BYTE_LIMIT / 16) + 1_024),
            "streamingFinalSeq": 0,
            "timestamp": 1.0,
            "parts": [{ "type": "text", "content": "large legacy message" }],
        });
        std::fs::write(
            &path,
            serde_json::to_vec(&value).expect("large legacy message JSON"),
        )
        .expect("large legacy message source");
        assert!(
            std::fs::metadata(&path)
                .expect("large legacy message metadata")
                .len()
                > IMPORT_BYTE_LIMIT as u64
        );
        path
    }

    fn write_large_legacy_workflow_source(
        root: &Path,
        source_root: &str,
        execution_id: &str,
    ) -> (std::path::PathBuf, u64) {
        use std::io::Write;

        let log_dir = root.join(source_root);
        std::fs::create_dir_all(&log_dir).expect("legacy workflow log directory");
        let path = log_dir.join(format!("{execution_id}.ndjson"));
        let mut writer =
            std::io::BufWriter::new(std::fs::File::create(&path).expect("workflow log source"));
        let padding = "workflow-additive-".repeat(256);
        let mut written = 0_u64;
        let mut record_count = 0_u64;
        while written <= IMPORT_BYTE_LIMIT as u64 {
            let record = serde_json::json!({
                "event": "execution_completed",
                "execution_id": execution_id,
                "total_token_usage": {
                    "inputTokens": record_count + 1,
                    "outputTokens": 1,
                },
                "timestamp": record_count as f64 + 1.0,
                "future_additive": padding,
            })
            .to_string();
            writer
                .write_all(record.as_bytes())
                .expect("legacy workflow record");
            writer
                .write_all(b"\n")
                .expect("legacy workflow record delimiter");
            written = written.saturating_add(record.len() as u64 + 1);
            record_count = record_count.saturating_add(1);
        }
        writer.flush().expect("flush legacy workflow source");
        assert!(
            std::fs::metadata(&path)
                .expect("large workflow source metadata")
                .len()
                > IMPORT_BYTE_LIMIT as u64
        );
        (path, record_count)
    }

    #[test]
    fn migration_streams_a_valid_source_larger_than_sixteen_mibibytes() {
        let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        enlarge_semantic_event_source_past_batch_limit(root.path());
        let database_path = root.path().join("staging.sqlite3");
        let mut connection = migration_connection(&database_path);

        import_legacy(
            &mut connection,
            root.path(),
            "streaming-source",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("a source larger than the migration batch bound must stream");

        assert_semantic_migration_projection(&connection);
        let raw_size: i64 = connection
            .query_row(
                "SELECT
                    (SELECT SUM(length(raw)) FROM legacy_raw_records
                     WHERE migration_id = 'streaming-source')
                    +
                    (SELECT COALESCE(SUM(length(raw)), 0) FROM legacy_raw_record_chunks
                     WHERE migration_id = 'streaming-source')",
                [],
                |row| row.get(0),
            )
            .expect("raw preservation size");
        assert!(raw_size > IMPORT_BYTE_LIMIT as i64);
    }

    #[test]
    fn f09_migration_streams_a_valid_large_legacy_message_source() {
        let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        write_large_legacy_message_source(root.path());
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        import_legacy(
            &mut connection,
            root.path(),
            "large-message-source",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("a valid message source larger than the batch bound must stream");

        let projection_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM message_projection
                 WHERE session_id = ?1 AND message_id = 'large-message-1'",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("large message projection count");
        assert_eq!(projection_count, 1);
    }

    fn assert_large_legacy_workflow_source_migrates(source_root: &str, migration_id: &str) {
        let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        let execution_id = format!("large-workflow-{source_root}");
        let (_, record_count) =
            write_large_legacy_workflow_source(root.path(), source_root, &execution_id);
        let mut connection = migration_connection(&root.path().join("staging.sqlite3"));

        import_legacy(
            &mut connection,
            root.path(),
            migration_id,
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("a valid workflow source larger than the batch bound must stream");

        let event_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE stream_id = ?1",
                params![format!("workflow:{execution_id}")],
                |row| row.get(0),
            )
            .expect("large workflow event count");
        assert_eq!(event_count, record_count as i64);
    }

    #[test]
    fn f09_migration_streams_large_workflow_execution_logs() {
        assert_large_legacy_workflow_source_migrates(
            "workflow_execution_logs",
            "large-workflow-execution-log",
        );
    }

    #[test]
    fn f09_migration_streams_large_workflow_event_logs() {
        assert_large_legacy_workflow_source_migrates(
            "workflow_event_logs",
            "large-workflow-event-log",
        );
    }

    #[test]
    fn chunked_source_restart_resumes_record_substep_without_duplicates() {
        let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        let source_size = enlarge_semantic_event_source_past_batch_limit(root.path());
        let database_path = root.path().join("staging.sqlite3");
        let mut connection = migration_connection(&database_path);

        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = import_legacy(
                &mut connection,
                root.path(),
                "streaming-restart",
                "generation-test",
                1_000,
                |connection| {
                    let checkpoint = connection
                        .query_row(
                            "SELECT checkpoint FROM local_store_migrations
                             WHERE migration_id = 'streaming-restart'",
                            [],
                            |row| row.get::<_, String>(0),
                        )
                        .ok()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
                    if checkpoint
                        .as_ref()
                        .and_then(|value| value.get("substep"))
                        .and_then(serde_json::Value::as_str)
                        == Some("raw_chunks")
                        && checkpoint
                            .as_ref()
                            .and_then(|value| value.get("source_record_ordinal"))
                            .and_then(serde_json::Value::as_u64)
                            == Some(2)
                    {
                        panic!("injected process crash between raw chunk substeps");
                    }
                },
            );
        }));
        assert!(
            crashed.is_err(),
            "fixture must cut the first boot mid-source"
        );
        let interrupted_checkpoint: String = connection
            .query_row(
                "SELECT checkpoint FROM local_store_migrations
                 WHERE migration_id = 'streaming-restart'",
                [],
                |row| row.get(0),
            )
            .expect("interrupted migration checkpoint");
        let interrupted_checkpoint: serde_json::Value =
            serde_json::from_str(&interrupted_checkpoint).expect("checkpoint JSON");
        assert!(interrupted_checkpoint.get("source_ordinal").is_some());
        assert_eq!(
            interrupted_checkpoint
                .get("source_record_ordinal")
                .and_then(serde_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(
            interrupted_checkpoint
                .get("substep")
                .and_then(serde_json::Value::as_str),
            Some("raw_chunks")
        );
        drop(connection);

        let mut reopened = migration_connection(&database_path);
        import_legacy(
            &mut reopened,
            root.path(),
            "streaming-restart",
            "generation-test",
            1_000,
            |_| {},
        )
        .expect("same migration resumes after the committed chunk checkpoint");

        assert_semantic_migration_projection(&reopened);
        let (chunk_count, distinct_chunks, preserved_bytes): (i64, i64, i64) = reopened
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT chunk_ordinal), SUM(length(raw))
                 FROM legacy_raw_record_chunks
                 WHERE migration_id = 'streaming-restart'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("resumed chunk parity");
        let expected_chunks = source_size.div_ceil(RAW_CHUNK_LIMIT as u64);
        assert_eq!(u64::try_from(chunk_count).ok(), Some(expected_chunks));
        assert_eq!(chunk_count, distinct_chunks, "no chunk may be duplicated");
        assert_eq!(u64::try_from(preserved_bytes).ok(), Some(source_size));
    }

    #[test]
    fn semantic_record_streaming_is_bounded_and_restart_has_no_event_terminal_or_projection_duplicates(
    ) {
        let _heavy_test_lock = crate::test_support::LOCAL_EVENT_STORE_HEAVY_TEST_LOCK.lock();
        const TURN_COUNT: u64 = 1_024;

        fn semantic_commits(connection: &Connection) -> Vec<(String, Vec<u8>, i64)> {
            let mut statement = connection
                .prepare(
                    "SELECT commit_id, result_hash, event_count
                     FROM logical_commits
                     WHERE operation_kind = 'migration'
                       AND commit_id LIKE 'migration-event-%'
                     ORDER BY commit_id ASC",
                )
                .expect("semantic commits");
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .expect("semantic commit rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("semantic commit values")
        }

        let root = tempfile::TempDir::new().expect("migration app data");
        semantic_migration_fixture(root.path());
        let source_size = replace_with_many_large_semantic_event_records(root.path(), TURN_COUNT);
        assert!(source_size > IMPORT_BYTE_LIMIT as u64);
        let database_path = root.path().join("staging.sqlite3");
        let mut connection = migration_connection(&database_path);
        let mut semantic_chunks = Vec::<(u64, u64, u64)>::new();

        let event_import_cut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = import_legacy(
                &mut connection,
                root.path(),
                "semantic-record-restart",
                "generation-test",
                1_000,
                |connection| {
                    let checkpoint = connection
                        .query_row(
                            "SELECT checkpoint FROM local_store_migrations
                             WHERE migration_id = 'semantic-record-restart'",
                            [],
                            |row| row.get::<_, String>(0),
                        )
                        .ok()
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
                    let Some(checkpoint) = checkpoint else {
                        return;
                    };
                    if let Some("semantic_events") = checkpoint
                        .get("substep")
                        .and_then(serde_json::Value::as_str)
                    {
                        let records = checkpoint
                            .get("semantic_chunk_record_count")
                            .and_then(serde_json::Value::as_u64)
                            .expect("semantic chunk record count");
                        let events = checkpoint
                            .get("semantic_chunk_event_count")
                            .and_then(serde_json::Value::as_u64)
                            .expect("semantic chunk event count");
                        let bytes = checkpoint
                            .get("semantic_chunk_decoded_bytes")
                            .and_then(serde_json::Value::as_u64)
                            .expect("semantic chunk decoded bytes");
                        semantic_chunks.push((records, events, bytes));
                        if checkpoint
                            .get("semantic_source_kind")
                            .and_then(serde_json::Value::as_str)
                            == Some("agent_session")
                            && checkpoint
                                .get("semantic_next_record_ordinal")
                                .and_then(serde_json::Value::as_u64)
                                == Some(256)
                        {
                            panic!("injected crash after a durable semantic event chunk");
                        }
                    }
                },
            );
        }));
        assert!(
            event_import_cut.is_err(),
            "fixture must cut semantic event import mid-source"
        );
        let (event_checkpoint, event_checkpoint_revision): (String, i64) = connection
            .query_row(
                "SELECT checkpoint, revision FROM local_store_migrations
                 WHERE migration_id = 'semantic-record-restart'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("interrupted semantic event checkpoint");
        let event_checkpoint: serde_json::Value =
            serde_json::from_str(&event_checkpoint).expect("semantic event checkpoint JSON");
        assert_eq!(
            event_checkpoint
                .get("substep")
                .and_then(serde_json::Value::as_str),
            Some("semantic_events")
        );
        assert_eq!(
            event_checkpoint
                .get("semantic_next_record_ordinal")
                .and_then(serde_json::Value::as_u64),
            Some(256)
        );
        assert_eq!(
            event_checkpoint
                .get("semantic_next_chunk_index")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        let first_event_commits = semantic_commits(&connection);
        assert!(!first_event_commits.is_empty());
        drop(connection);

        let mut materializing = migration_connection(&database_path);
        let mut first_resumed_event_record = None::<u64>;
        let materialization_cut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = import_legacy(
                &mut materializing,
                root.path(),
                "semantic-record-restart",
                "generation-test",
                1_000,
                |connection| {
                    let checkpoint = connection
                        .query_row(
                            "SELECT checkpoint, revision FROM local_store_migrations
                             WHERE migration_id = 'semantic-record-restart'",
                            [],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                        )
                        .ok();
                    let Some((checkpoint, revision)) = checkpoint else {
                        return;
                    };
                    if revision <= event_checkpoint_revision {
                        return;
                    }
                    let checkpoint = serde_json::from_str::<serde_json::Value>(&checkpoint)
                        .expect("resumed semantic checkpoint JSON");
                    match checkpoint
                        .get("substep")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("semantic_events")
                            if checkpoint
                                .get("semantic_source_kind")
                                .and_then(serde_json::Value::as_str)
                                == Some("agent_session") =>
                        {
                            let next_record = checkpoint
                                .get("semantic_next_record_ordinal")
                                .and_then(serde_json::Value::as_u64)
                                .expect("resumed semantic event record");
                            first_resumed_event_record.get_or_insert(next_record);
                            let records = checkpoint
                                .get("semantic_chunk_record_count")
                                .and_then(serde_json::Value::as_u64)
                                .expect("semantic chunk record count");
                            let events = checkpoint
                                .get("semantic_chunk_event_count")
                                .and_then(serde_json::Value::as_u64)
                                .expect("semantic chunk event count");
                            let bytes = checkpoint
                                .get("semantic_chunk_decoded_bytes")
                                .and_then(serde_json::Value::as_u64)
                                .expect("semantic chunk decoded bytes");
                            semantic_chunks.push((records, events, bytes));
                        }
                        Some("semantic_session_events")
                            if checkpoint
                                .get("semantic_next_record_ordinal")
                                .and_then(serde_json::Value::as_u64)
                                == Some(256) =>
                        {
                            panic!(
                                "injected crash after a durable semantic materialization record"
                            );
                        }
                        _ => {}
                    }
                },
            );
        }));
        assert!(
            materialization_cut.is_err(),
            "fixture must cut materialization mid-source"
        );
        assert_eq!(
            first_resumed_event_record,
            Some(512),
            "the durable 256-record event chunk must not be transactionally replayed"
        );
        assert!(
            semantic_chunks.len() > 1,
            "semantic import must use several chunks"
        );
        assert!(semantic_chunks.iter().all(|(records, events, bytes)| {
            *records <= IMPORT_RECORD_LIMIT as u64
                && *events <= SEMANTIC_EVENT_CHUNK_LIMIT as u64
                && *bytes <= IMPORT_BYTE_LIMIT as u64
        }));
        assert_eq!(
            semantic_chunks.iter().map(|chunk| chunk.0).max(),
            Some(IMPORT_RECORD_LIMIT as u64),
            "the production observer must see the configured record peak"
        );
        let event_commits_after_event_resume = semantic_commits(&materializing);
        assert!(first_event_commits.iter().all(|commit| {
            event_commits_after_event_resume
                .iter()
                .any(|resumed| resumed == commit)
        }));
        let (interrupted_checkpoint, materialization_checkpoint_revision): (String, i64) =
            materializing
                .query_row(
                    "SELECT checkpoint, revision FROM local_store_migrations
                 WHERE migration_id = 'semantic-record-restart'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("interrupted semantic checkpoint");
        let interrupted_checkpoint: serde_json::Value =
            serde_json::from_str(&interrupted_checkpoint).expect("semantic checkpoint JSON");
        assert_eq!(
            interrupted_checkpoint
                .get("substep")
                .and_then(serde_json::Value::as_str),
            Some("semantic_session_events")
        );
        assert!(
            interrupted_checkpoint
                .get("semantic_source_ordinal")
                .and_then(serde_json::Value::as_u64)
                .is_some(),
            "the durable checkpoint binds the inventory source"
        );
        assert_eq!(
            interrupted_checkpoint
                .get("semantic_next_record_ordinal")
                .and_then(serde_json::Value::as_u64),
            Some(256)
        );
        let event_commits_before_resume = semantic_commits(&materializing);
        let projection_revision_before_resume: i64 = materializing
            .query_row(
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("prefix projection revision");
        drop(materializing);

        let mut reopened = migration_connection(&database_path);
        let mut resumed_semantic_event_callbacks = 0_u64;
        let mut first_resumed_materialization_record = None::<u64>;
        import_legacy(
            &mut reopened,
            root.path(),
            "semantic-record-restart",
            "generation-test",
            1_000,
            |connection| {
                let checkpoint = connection
                    .query_row(
                        "SELECT checkpoint, revision FROM local_store_migrations
                         WHERE migration_id = 'semantic-record-restart'",
                        [],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .ok();
                let Some((checkpoint, revision)) = checkpoint else {
                    return;
                };
                if revision <= materialization_checkpoint_revision {
                    return;
                }
                let checkpoint = serde_json::from_str::<serde_json::Value>(&checkpoint)
                    .expect("final resumed semantic checkpoint JSON");
                match checkpoint
                    .get("substep")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("semantic_events") => resumed_semantic_event_callbacks += 1,
                    Some("semantic_session_events") => {
                        let next = checkpoint
                            .get("semantic_next_record_ordinal")
                            .and_then(serde_json::Value::as_u64)
                            .expect("resumed semantic materialization ordinal");
                        first_resumed_materialization_record.get_or_insert(next);
                    }
                    _ => {}
                }
            },
        )
        .expect("semantic record checkpoint resumes with the same identity");
        assert_eq!(
            resumed_semantic_event_callbacks, 0,
            "completed semantic event chunks must not be transactionally replayed"
        );
        assert_eq!(
            first_resumed_materialization_record,
            Some(257),
            "the committed 256-record fold prefix is parsed only to rebuild accumulator state"
        );
        let event_commits_after_resume = {
            let mut statement = reopened
                .prepare(
                    "SELECT commit_id, result_hash, event_count
                     FROM logical_commits
                     WHERE operation_kind = 'migration'
                       AND commit_id LIKE 'migration-event-%'
                     ORDER BY commit_id ASC",
                )
                .expect("resumed semantic commits");
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .expect("resumed semantic commit rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("resumed semantic commit values")
        };
        assert_eq!(event_commits_after_resume, event_commits_before_resume);
        let projection_revision_after_resume: i64 = reopened
            .query_row(
                "SELECT revision FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("resumed projection revision");
        assert_eq!(
            projection_revision_after_resume,
            projection_revision_before_resume
        );

        let expected_event_count = i64::try_from(TURN_COUNT * 2).expect("event count");
        let (events, distinct_events, terminals, distinct_terminals, projections): (
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = reopened
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM events
                     WHERE stream_id = ?1),
                    (SELECT COUNT(DISTINCT event_id) FROM events
                     WHERE stream_id = ?1),
                    (SELECT COUNT(*) FROM terminal_records
                     WHERE session_id = ?2),
                    (SELECT COUNT(DISTINCT turn_id) FROM terminal_records
                     WHERE session_id = ?2),
                    (SELECT COUNT(*) FROM session_projection
                     WHERE session_id = ?2)",
                params![
                    format!("agent-session:{SEMANTIC_SESSION_ID}"),
                    SEMANTIC_SESSION_ID
                ],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("resumed semantic identity counts");
        assert_eq!(
            (events, distinct_events),
            (expected_event_count, expected_event_count)
        );
        assert_eq!(
            (terminals, distinct_terminals),
            (
                i64::try_from(TURN_COUNT).expect("terminal count"),
                i64::try_from(TURN_COUNT).expect("terminal count"),
            )
        );
        assert_eq!(projections, 1);

        let projection: String = reopened
            .query_row(
                "SELECT projection FROM session_projection WHERE session_id = ?1",
                params![SEMANTIC_SESSION_ID],
                |row| row.get(0),
            )
            .expect("resumed semantic projection");
        let projection = decode_canonical_agent_session_projection(&projection);
        assert_eq!(projection.meta.last_turn_id, Some(TURN_COUNT));
        assert_eq!(projection.reducer_events.len(), 2);
    }
}
