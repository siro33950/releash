//! Versioned schema for the fixed-path permanent local event store.

use crate::infrastructure::local_event_store_schema;
use rusqlite::Connection;

use super::fault::{FaultInjector, InitialCreateFaultPoint};

/// Minimum SQLite version containing the WAL-reset corruption fix.
pub const APPLICATION_ID: i32 = 0x524C_5348;
pub const CURRENT_SCHEMA_VERSION: i64 = 8;

pub const CURRENT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS logical_commits (
    commit_id TEXT PRIMARY KEY,
    installation_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL
        CHECK (operation_kind IN (
            'send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit',
            'recovery', 'user_mutation', 'shutdown_target',
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
    UNIQUE (installation_id, operation_kind, idempotency_key),
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

CREATE TABLE IF NOT EXISTS session_projection (
    session_id TEXT PRIMARY KEY,
    projection TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    workspace_identity TEXT,
    public_list_kind TEXT
        CHECK (public_list_kind IS NULL OR public_list_kind IN ('active', 'closed', 'archived')),
    public_sort_key_bits INTEGER,
    public_summary TEXT,
    CHECK (
        (public_list_kind IS NULL AND public_sort_key_bits IS NULL AND public_summary IS NULL)
        OR
        (workspace_identity IS NOT NULL AND public_list_kind IS NOT NULL
         AND public_sort_key_bits IS NOT NULL AND public_summary IS NOT NULL)
    )
);

"#;

/// The unified-node fact log: one normalized table, one row per pure fact.
/// No transition events and no status columns; state is derived by the
/// domain fold on the read side. Columns beyond the fact identity exist for
/// tree / node / kind narrowing only, so `event_type` carries no CHECK (the
/// vocabulary is owned by the domain, not duplicated into SQL).
const NODE_EVENTS_SCHEMA_V7: &str = r#"
CREATE TABLE IF NOT EXISTS node_events (
    tree_id TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    node_execution_id TEXT NOT NULL,
    parent_id TEXT,
    node_name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('session', 'command', 'fanout', 'sequence')),
    attempt INTEGER NOT NULL CHECK (attempt >= 1),
    event_type TEXT NOT NULL,
    session_id TEXT,
    detail TEXT NOT NULL,
    timestamp INTEGER NOT NULL CHECK (timestamp >= 0),
    PRIMARY KEY (tree_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_node_events_node
    ON node_events (node_execution_id, seq);
CREATE INDEX IF NOT EXISTS idx_node_events_kind
    ON node_events (kind, tree_id);
CREATE INDEX IF NOT EXISTS idx_node_events_session
    ON node_events (session_id, tree_id, seq)
    WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_node_events_event_type
    ON node_events (event_type, node_execution_id, seq);
"#;

const SESSION_PROJECTION_TABLE_V3: &str = r#"
CREATE TABLE IF NOT EXISTS session_projection (
    session_id TEXT PRIMARY KEY,
    projection TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    workspace_identity TEXT,
    public_list_kind TEXT
        CHECK (public_list_kind IS NULL OR public_list_kind IN ('active', 'closed', 'archived')),
    public_sort_key_bits INTEGER,
    public_summary TEXT,
    CHECK (
        (public_list_kind IS NULL AND public_sort_key_bits IS NULL AND public_summary IS NULL)
        OR
        (workspace_identity IS NOT NULL AND public_list_kind IS NOT NULL
         AND public_sort_key_bits IS NOT NULL AND public_summary IS NOT NULL)
    )
);
"#;

pub struct InitialStoreMetadata<'a> {
    pub installation_id: &'a str,
    pub created_at_ms: i64,
}

pub fn initialize_schema(
    connection: &Connection,
    metadata: &InitialStoreMetadata<'_>,
    fault: &FaultInjector,
) -> Result<(), rusqlite::Error> {
    local_event_store_schema::transaction(connection, |connection| {
        connection.execute_batch(CURRENT_SCHEMA)?;
        connection.execute_batch(SESSION_PROJECTION_TABLE_V3)?;
        local_event_store_schema::create_store_metadata_v8(connection)?;
        connection.execute_batch(NODE_EVENTS_SCHEMA_V7)?;
        connection.execute(
            "INSERT INTO store_metadata (
                id, schema_version, installation_id, created_at_ms,
                next_global_sequence, health
             ) VALUES (1, 8, ?1, ?2, 1, 'ok')",
            rusqlite::params![metadata.installation_id, metadata.created_at_ms,],
        )?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        if fault.take_initial_create_fault(InitialCreateFaultPoint::BeforeInitializationCommit) {
            #[cfg(any(test, feature = "test-support"))]
            fault.crash_initial_create_process_if_armed(
                InitialCreateFaultPoint::BeforeInitializationCommit,
            );
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    })?;
    if fault.take_initial_create_fault(InitialCreateFaultPoint::AfterInitializationCommitReplyLoss)
    {
        #[cfg(any(test, feature = "test-support"))]
        fault.crash_initial_create_process_if_armed(
            InitialCreateFaultPoint::AfterInitializationCommitReplyLoss,
        );
        return Err(rusqlite::Error::InvalidQuery);
    }
    finish_schema_admission(connection)
}

fn finish_schema_admission(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_node_events_tree_event_type
         ON node_events (tree_id, event_type, seq);",
    )?;
    connection.execute_batch("PRAGMA secure_delete = ON;")?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

pub fn validate_current_schema_marker(connection: &Connection) -> Result<(), rusqlite::Error> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if application_id != i64::from(APPLICATION_ID) || user_version != CURRENT_SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn validate_current_schema(connection: &Connection) -> Result<(), rusqlite::Error> {
    validate_current_schema_marker(connection)?;
    let metadata_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM store_metadata", [], |row| row.get(0))?;
    if metadata_count != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let metadata: (i64, String) = connection.query_row(
        "SELECT schema_version, installation_id FROM store_metadata WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if metadata.0 != CURRENT_SCHEMA_VERSION || uuid::Uuid::parse_str(&metadata.1).is_err() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    require_exact_columns(
        connection,
        "store_metadata",
        &[
            "id",
            "schema_version",
            "installation_id",
            "created_at_ms",
            "next_global_sequence",
            "health",
        ],
    )?;
    require_exact_columns(
        connection,
        "logical_commits",
        &[
            "commit_id",
            "installation_id",
            "operation_kind",
            "idempotency_key",
            "payload_hash",
            "state",
            "first_global_sequence",
            "last_global_sequence",
            "event_count",
            "mutation_count",
            "stream_heads_json",
            "result_hash",
            "committed_at_ms",
        ],
    )?;
    require_exact_columns(
        connection,
        "session_projection",
        &[
            "session_id",
            "projection",
            "revision",
            "commit_id",
            "workspace_identity",
            "public_list_kind",
            "public_sort_key_bits",
            "public_summary",
        ],
    )?;
    require_exact_columns(
        connection,
        "node_events",
        &[
            "tree_id",
            "seq",
            "node_execution_id",
            "parent_id",
            "node_name",
            "kind",
            "attempt",
            "event_type",
            "session_id",
            "detail",
            "timestamp",
        ],
    )?;
    for index in [
        "idx_node_events_node",
        "idx_node_events_kind",
        "idx_node_events_session",
        "idx_node_events_event_type",
    ] {
        require_index(connection, index)?;
    }
    for table in [
        "operation_bindings",
        "caller_attempts",
        "operation_records",
        "pending_obligations",
        "obligations",
        "recovery_action_attempts",
        "shutdown_targets",
        "shutdown_recovery_snapshots",
        "shutdown_plans",
        "message_projection",
        "terminal_records",
        "stop_resolutions",
        "workflow_executions",
        "workflow_execution_nodes",
    ] {
        require_schema_object_absent(connection, "table", table)?;
    }
    require_schema_object_absent(connection, "index", "idx_message_projection_ordinal")?;
    require_schema_object_absent(connection, "index", "idx_session_projection_public_list")?;
    require_schema_object_absent(connection, "index", "idx_session_projection_public_node")?;
    let divergent_identity_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM logical_commits
         WHERE installation_id <> (SELECT installation_id FROM store_metadata WHERE id = 1)",
        [],
        |row| row.get(0),
    )?;
    if divergent_identity_count != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    require_foreign_key_integrity(connection)?;
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn require_exact_columns(
    connection: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), rusqlite::Error> {
    let columns = table_columns(connection, table)?;
    if columns.len() != expected.len()
        || !columns
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual == expected)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn require_index(connection: &Connection, index: &str) -> Result<(), rusqlite::Error> {
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'index' AND name = ?1 AND sql IS NOT NULL",
        [index],
        |row| row.get(0),
    )?;
    if exists != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn require_schema_object_absent(
    connection: &Connection,
    object_type: &str,
    name: &str,
) -> Result<(), rusqlite::Error> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = ?1 AND name = ?2",
        rusqlite::params![object_type, name],
        |row| row.get(0),
    )?;
    if count != 0 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn require_foreign_key_integrity(connection: &Connection) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}
