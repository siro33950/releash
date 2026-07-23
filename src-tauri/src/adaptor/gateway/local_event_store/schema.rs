//! SQLite schema version 1 for the permanent local event store.
//!
//! Twenty tables plus the CHECK / foreign-key constraints and the design's
//! index set only. Public query indexes are limited to operation identity,
//! terminal unique key, pending ordered key + owner / partition / shutdown
//! association, shutdown plan / target, and event stream / global sequence.

use rusqlite::Connection;

/// Minimum SQLite library version required at compile / startup.
pub const MIN_SQLITE_VERSION_NUMBER: i32 = 3_045_000;

pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS store_metadata (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    store_id TEXT NOT NULL,
    generation_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    cursor_hmac_key BLOB NOT NULL CHECK (length(cursor_hmac_key) = 32),
    operation_binding_hmac_key BLOB NOT NULL
        CHECK (length(operation_binding_hmac_key) = 32),
    boot_id TEXT NOT NULL,
    next_global_sequence INTEGER NOT NULL CHECK (next_global_sequence >= 1),
    health TEXT NOT NULL CHECK (health IN ('ok', 'recovering')),
    current_shutdown_plan_id TEXT,
    current_shutdown_epoch INTEGER
        CHECK (current_shutdown_epoch IS NULL OR current_shutdown_epoch >= 0),
    shutdown_pointer_revision INTEGER NOT NULL CHECK (shutdown_pointer_revision >= 0),
    retiring_shutdown_plan_id TEXT,
    retiring_shutdown_epoch INTEGER
        CHECK (retiring_shutdown_epoch IS NULL OR retiring_shutdown_epoch >= 0),
    shutdown_retiring_revision INTEGER NOT NULL DEFAULT 0
        CHECK (shutdown_retiring_revision >= 0),
    CHECK ((current_shutdown_plan_id IS NULL) = (current_shutdown_epoch IS NULL)),
    CHECK ((retiring_shutdown_plan_id IS NULL) = (retiring_shutdown_epoch IS NULL))
);

CREATE TABLE IF NOT EXISTS logical_commits (
    commit_id TEXT PRIMARY KEY,
    generation_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL
        CHECK (operation_kind IN (
            'send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit',
            'recovery', 'migration', 'user_mutation', 'shutdown_target',
            'operation_progress', 'projection', 'workflow'
        )),
    idempotency_key TEXT NOT NULL,
    payload_hash BLOB NOT NULL CHECK (length(payload_hash) = 32),
    state TEXT NOT NULL CHECK (state IN ('preparing', 'sealed')),
    first_global_sequence INTEGER CHECK (first_global_sequence IS NULL OR first_global_sequence >= 1),
    last_global_sequence INTEGER CHECK (last_global_sequence IS NULL OR last_global_sequence >= 1),
    event_count INTEGER NOT NULL CHECK (event_count >= 0),
    mutation_count INTEGER NOT NULL CHECK (mutation_count >= 0),
    stream_heads_json TEXT NOT NULL,
    result_hash BLOB CHECK (result_hash IS NULL OR length(result_hash) = 32),
    committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms >= 0),
    UNIQUE (generation_id, operation_kind, idempotency_key),
    CHECK ((first_global_sequence IS NULL) = (last_global_sequence IS NULL))
);

CREATE TABLE IF NOT EXISTS stream_heads (
    stream_id TEXT PRIMARY KEY,
    head INTEGER NOT NULL CHECK (head >= 0),
    updated_commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS events (
    global_sequence INTEGER PRIMARY KEY CHECK (global_sequence >= 1),
    event_id TEXT NOT NULL UNIQUE,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    stream_id TEXT NOT NULL,
    stream_sequence INTEGER NOT NULL CHECK (stream_sequence >= 1),
    event_type TEXT NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version >= 1),
    occurred_at TEXT NOT NULL,
    payload BLOB NOT NULL,
    payload_sha256 BLOB NOT NULL CHECK (length(payload_sha256) = 32),
    UNIQUE (stream_id, stream_sequence)
);

CREATE TABLE IF NOT EXISTS operation_bindings (
    principal TEXT NOT NULL,
    generation_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    caller_request_id TEXT NOT NULL,
    scope_id TEXT,
    operation_id TEXT NOT NULL,
    binding_hmac BLOB NOT NULL CHECK (length(binding_hmac) = 32),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (principal, generation_id, kind, caller_request_id)
);

CREATE TABLE IF NOT EXISTS caller_attempts (
    principal TEXT NOT NULL,
    generation_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    caller_request_id TEXT NOT NULL,
    scope_id TEXT,
    command_hash BLOB NOT NULL CHECK (length(command_hash) = 32),
    sealed_command BLOB NOT NULL,
    resolution TEXT NOT NULL
        CHECK (resolution IN ('pending', 'accepted', 'rejected_before_commit', 'cleared')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (principal, generation_id, kind, caller_request_id)
);

CREATE TABLE IF NOT EXISTS operation_records (
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    operation_id TEXT NOT NULL,
    receipt TEXT NOT NULL,
    latest_status TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (kind, operation_id)
);

CREATE TABLE IF NOT EXISTS session_projection (
    session_id TEXT PRIMARY KEY,
    projection TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS message_projection (
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    message_ordinal INTEGER NOT NULL CHECK (message_ordinal > 0),
    projection TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (session_id, message_id),
    UNIQUE (session_id, message_ordinal)
);

CREATE TABLE IF NOT EXISTS terminal_records (
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    terminal_identity TEXT NOT NULL,
    result TEXT NOT NULL,
    participant_digest BLOB NOT NULL CHECK (length(participant_digest) = 32),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (session_id, turn_id)
);

CREATE TABLE IF NOT EXISTS stop_resolutions (
    stop_operation_id TEXT PRIMARY KEY,
    resolution TEXT NOT NULL CHECK (resolution IN ('succeeded', 'superseded')),
    detail TEXT NOT NULL,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS obligations (
    obligation_id TEXT PRIMARY KEY,
    record TEXT NOT NULL,
    pending INTEGER NOT NULL CHECK (pending IN (0, 1)),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS pending_obligations (
    ordered_key TEXT PRIMARY KEY,
    obligation_id TEXT NOT NULL UNIQUE REFERENCES obligations (obligation_id),
    owner TEXT NOT NULL,
    partition TEXT NOT NULL
        CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
    shutdown_plan_id TEXT,
    shutdown_epoch INTEGER CHECK (shutdown_epoch IS NULL OR shutdown_epoch >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    CHECK ((shutdown_plan_id IS NULL) = (shutdown_epoch IS NULL))
);

CREATE TABLE IF NOT EXISTS recovery_action_attempts (
    action_id TEXT PRIMARY KEY,
    binding_hash BLOB NOT NULL CHECK (length(binding_hash) = 32),
    attempt TEXT NOT NULL,
    completed TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS shutdown_plans (
    plan_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'preparing', 'prepared', 'activated', 'quiescing',
        'completed', 'failed', 'cancelled', 'reconciliation_required'
    )),
    summary TEXT NOT NULL,
    details_state TEXT NOT NULL CHECK (details_state IN ('available', 'compacted')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (plan_id, epoch)
);

CREATE TABLE IF NOT EXISTS shutdown_targets (
    plan_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (plan_id, epoch, ordinal),
    FOREIGN KEY (plan_id, epoch) REFERENCES shutdown_plans (plan_id, epoch)
);

CREATE TABLE IF NOT EXISTS shutdown_recovery_snapshots (
    plan_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    partition TEXT NOT NULL
        CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (plan_id, epoch, partition, ordinal),
    FOREIGN KEY (plan_id, epoch) REFERENCES shutdown_plans (plan_id, epoch)
);

CREATE TABLE IF NOT EXISTS shutdown_compact_archives (
    plan_id TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    archive TEXT NOT NULL,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (plan_id, epoch)
);

CREATE TABLE IF NOT EXISTS local_store_migrations (
    migration_id TEXT PRIMARY KEY,
    phase TEXT NOT NULL CHECK (phase IN (
        'inspecting_source', 'importing', 'verifying', 'activating', 'failed'
    )),
    source_inventory_hash BLOB NOT NULL CHECK (length(source_inventory_hash) = 32),
    checkpoint TEXT NOT NULL,
    parity TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS legacy_source_inventory (
    migration_id TEXT NOT NULL REFERENCES local_store_migrations (migration_id),
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal >= 0),
    source_path TEXT NOT NULL,
    source_size TEXT NOT NULL,
    modified_ms TEXT NOT NULL,
    raw_sha256 BLOB NOT NULL CHECK (length(raw_sha256) = 32),
    record_count INTEGER NOT NULL CHECK (record_count > 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (migration_id, source_ordinal),
    UNIQUE (migration_id, source_path)
);

CREATE TABLE IF NOT EXISTS legacy_raw_records (
    migration_id TEXT NOT NULL REFERENCES local_store_migrations (migration_id),
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal >= 0),
    source_path TEXT NOT NULL,
    source_size TEXT NOT NULL,
    modified_ms TEXT NOT NULL,
    record_count INTEGER NOT NULL CHECK (record_count > 0),
    raw BLOB NOT NULL,
    raw_sha256 BLOB NOT NULL CHECK (length(raw_sha256) = 32),
    PRIMARY KEY (migration_id, source_ordinal)
);

-- Sources larger than one migration batch retain their exact bytes as
-- independently committed chunks.  The parent row above remains the
-- one-record-per-source parity anchor and carries an empty `raw` sentinel for
-- chunked sources; readers must verify the ordered chunks against its full
-- source digest before treating the migration as activated.
CREATE TABLE IF NOT EXISTS legacy_raw_record_chunks (
    migration_id TEXT NOT NULL,
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal >= 0),
    chunk_ordinal INTEGER NOT NULL CHECK (chunk_ordinal >= 0),
    source_offset TEXT NOT NULL,
    raw BLOB NOT NULL CHECK (length(raw) > 0 AND length(raw) <= 4194304),
    raw_sha256 BLOB NOT NULL CHECK (length(raw_sha256) = 32),
    PRIMARY KEY (migration_id, source_ordinal, chunk_ordinal),
    FOREIGN KEY (migration_id, source_ordinal)
        REFERENCES legacy_raw_records (migration_id, source_ordinal)
);

CREATE TABLE IF NOT EXISTS migration_quit_flights (
    operation_id TEXT PRIMARY KEY,
    migration_id TEXT NOT NULL UNIQUE
        REFERENCES local_store_migrations (migration_id),
    migration_revision INTEGER NOT NULL CHECK (migration_revision >= 0),
    checkpoint TEXT NOT NULL,
    accepted_boot_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE INDEX IF NOT EXISTS idx_pending_obligations_partition
    ON pending_obligations (partition, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_owner
    ON pending_obligations (owner, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_shutdown
    ON pending_obligations (shutdown_plan_id, shutdown_epoch, ordered_key);
CREATE INDEX IF NOT EXISTS idx_shutdown_plans_details_state
    ON shutdown_plans (details_state);
CREATE INDEX IF NOT EXISTS idx_legacy_raw_records_source_path
    ON legacy_raw_records (source_path, migration_id DESC);
CREATE INDEX IF NOT EXISTS idx_legacy_raw_record_chunks_source
    ON legacy_raw_record_chunks (migration_id, source_ordinal, chunk_ordinal);
"#;

/// Apply schema version 1. Must run under the exclusive writer lock before
/// normal admission opens.
pub fn apply_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(SCHEMA_V1)?;
    connection.execute_batch("PRAGMA secure_delete = ON;")?;
    let message_projection_columns = connection
        .prepare("PRAGMA table_info(message_projection)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !message_projection_columns
        .iter()
        .any(|column| column == "message_ordinal")
    {
        // Old SQLite authority rows predate an explicit semantic position.
        // Backfill from the gateway-owned stored message timestamp with the
        // stable message identity as a tie-breaker.  SQLite rowid and legacy
        // path lexical order are deliberately not used as semantic order.
        connection.execute(
            "ALTER TABLE message_projection ADD COLUMN message_ordinal INTEGER",
            [],
        )?;
        let mut rows = connection
            .prepare("SELECT session_id, message_id, projection FROM message_projection")?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_by(|left, right| {
            let timestamp = |raw: &str| {
                serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|value| value.get("timestamp").and_then(serde_json::Value::as_f64))
                    .filter(|value| value.is_finite())
                    .map(f64::to_bits)
            };
            left.0
                .cmp(&right.0)
                .then_with(|| timestamp(&left.2).cmp(&timestamp(&right.2)))
                .then_with(|| left.1.cmp(&right.1))
        });
        let mut current_session = None::<String>;
        let mut ordinal = 0_i64;
        for (session_id, message_id, _) in rows {
            if current_session.as_deref() != Some(session_id.as_str()) {
                current_session = Some(session_id.clone());
                ordinal = 1;
            } else {
                ordinal = ordinal.checked_add(1).ok_or_else(|| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                        "message ordinal overflow during schema backfill",
                    )))
                })?;
            }
            connection.execute(
                "UPDATE message_projection SET message_ordinal = ?3
                 WHERE session_id = ?1 AND message_id = ?2",
                rusqlite::params![session_id, message_id, ordinal],
            )?;
        }
    }
    connection.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_message_projection_ordinal
         ON message_projection (session_id, message_ordinal)",
        [],
    )?;
    let has_scope_id = {
        let mut statement = connection.prepare("PRAGMA table_info(caller_attempts)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        columns
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|column| column == "scope_id")
    };
    if !has_scope_id {
        connection.execute("ALTER TABLE caller_attempts ADD COLUMN scope_id TEXT", [])?;
    }
    let metadata_columns = {
        let mut statement = connection.prepare("PRAGMA table_info(store_metadata)")?;
        let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
        columns.collect::<Result<Vec<_>, _>>()?
    };
    if !metadata_columns
        .iter()
        .any(|column| column == "retiring_shutdown_plan_id")
    {
        connection.execute(
            "ALTER TABLE store_metadata ADD COLUMN retiring_shutdown_plan_id TEXT",
            [],
        )?;
    }
    if !metadata_columns
        .iter()
        .any(|column| column == "retiring_shutdown_epoch")
    {
        connection.execute(
            "ALTER TABLE store_metadata ADD COLUMN retiring_shutdown_epoch INTEGER",
            [],
        )?;
    }
    if !metadata_columns
        .iter()
        .any(|column| column == "shutdown_retiring_revision")
    {
        connection.execute(
            "ALTER TABLE store_metadata ADD COLUMN shutdown_retiring_revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_caller_attempts_scope
         ON caller_attempts (principal, generation_id, scope_id, kind, caller_request_id)",
        [],
    )?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_caller_attempts_pending_kind
         ON caller_attempts (generation_id, kind, resolution, principal, caller_request_id)",
        [],
    )?;
    connection.execute(
        "CREATE INDEX IF NOT EXISTS idx_operation_bindings_operation
         ON operation_bindings (generation_id, kind, operation_id, principal, caller_request_id)",
        [],
    )?;
    // Pre-encryption builds stored raw exact commands despite the historical
    // `sealed_command` name. They cannot be safely retried, so invalidate the
    // payload in place before normal admission. Binding/status records remain
    // available for deterministic operation readback.
    connection.execute(
        "UPDATE caller_attempts
         SET sealed_command = X'', resolution = 'cleared',
             revision = CASE WHEN revision < 9223372036854775807 THEN revision + 1 ELSE revision END
         WHERE length(sealed_command) > 0
           AND substr(sealed_command, 1, 5) <> X'524C534131'",
        [],
    )?;
    // The writer lock is exclusive during schema admission, so no reader can
    // retain the pre-encryption WAL. Truncation prevents an invalidated raw
    // command from surviving in a stale WAL frame.
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}
