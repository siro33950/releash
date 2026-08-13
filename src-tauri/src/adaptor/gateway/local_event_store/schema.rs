//! Versioned schema for the fixed-path permanent local event store.

use rusqlite::Connection;

use super::fault::{FaultInjector, InitialCreateFaultPoint};

#[cfg(test)]
#[path = "schema_test.rs"]
mod schema_test;

/// Minimum SQLite version containing the WAL-reset corruption fix.
pub const MIN_SQLITE_VERSION_NUMBER: i32 = 3_051_003;
pub const APPLICATION_ID: i32 = 0x524C_5348;
pub const CURRENT_SCHEMA_VERSION: i64 = 5;

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

CREATE TABLE IF NOT EXISTS operation_bindings (
    principal TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    caller_request_id TEXT NOT NULL,
    scope_id TEXT,
    operation_id TEXT NOT NULL,
    binding_hmac BLOB NOT NULL CHECK (length(binding_hmac) = 32),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (principal, installation_id, kind, caller_request_id)
);

CREATE TABLE IF NOT EXISTS caller_attempts (
    principal TEXT NOT NULL,
    installation_id TEXT NOT NULL,
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
    PRIMARY KEY (principal, installation_id, kind, caller_request_id)
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
    shutdown_id TEXT,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
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
    shutdown_id TEXT PRIMARY KEY,
    phase TEXT NOT NULL CHECK (phase IN (
        'prepared', 'activated', 'quiescing',
        'completed', 'failed', 'cancelled', 'reconciliation_required'
    )),
    summary TEXT NOT NULL,
    details_state TEXT NOT NULL CHECK (details_state IN ('available', 'compacted')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS shutdown_targets (
    shutdown_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (shutdown_id, ordinal),
    FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans (shutdown_id)
);

CREATE TABLE IF NOT EXISTS shutdown_recovery_snapshots (
    shutdown_id TEXT NOT NULL,
    partition TEXT NOT NULL
        CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (shutdown_id, ordinal),
    FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans (shutdown_id)
);

CREATE INDEX IF NOT EXISTS idx_pending_obligations_partition
    ON pending_obligations (partition, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_owner
    ON pending_obligations (owner, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_shutdown
    ON pending_obligations (shutdown_id, ordered_key);
CREATE INDEX IF NOT EXISTS idx_shutdown_plans_details_state
    ON shutdown_plans (details_state);
CREATE INDEX IF NOT EXISTS idx_caller_attempts_scope
    ON caller_attempts (principal, installation_id, scope_id, kind, caller_request_id);
CREATE INDEX IF NOT EXISTS idx_caller_attempts_pending_kind
    ON caller_attempts (installation_id, kind, resolution, principal, caller_request_id);
CREATE INDEX IF NOT EXISTS idx_operation_bindings_operation
    ON operation_bindings (installation_id, kind, operation_id, principal, caller_request_id);
"#;

const WORKSPACE_QUERY_RECORDS_V3: &str = r#"
CREATE TABLE IF NOT EXISTS workflow_executions (
    execution_id TEXT PRIMARY KEY,
    workspace_identity TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('running', 'waiting_approval', 'completed', 'failed', 'aborted', 'interrupted')
    ),
    list_kind TEXT NOT NULL CHECK (list_kind IN ('active', 'terminal')),
    sort_at_bits INTEGER NOT NULL,
    record_schema TEXT NOT NULL CHECK (record_schema = 'workflow_execution_record_v1'),
    record TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK (source_revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);
CREATE TABLE IF NOT EXISTS workflow_execution_nodes (
    execution_id TEXT NOT NULL REFERENCES workflow_executions (execution_id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    parent_id TEXT,
    sibling_order INTEGER NOT NULL CHECK (sibling_order >= 0),
    session_id TEXT,
    node_execution_id TEXT,
    record_schema TEXT NOT NULL CHECK (record_schema = 'workflow_execution_node_record_v1'),
    tree_record TEXT NOT NULL,
    detail_record TEXT NOT NULL,
    source_revision INTEGER NOT NULL CHECK (source_revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (execution_id, node_id),
    UNIQUE (execution_id, node_execution_id)
);
CREATE INDEX IF NOT EXISTS idx_workflow_executions_workspace_list
    ON workflow_executions (
        workspace_identity, list_kind, sort_at_bits DESC, execution_id
    );
CREATE INDEX IF NOT EXISTS idx_workflow_executions_global_list
    ON workflow_executions (list_kind, sort_at_bits DESC, execution_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_execution_nodes_node
    ON workflow_execution_nodes (node_id);
CREATE INDEX IF NOT EXISTS idx_workflow_execution_nodes_occurrence
    ON workflow_execution_nodes (execution_id, sibling_order, node_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_execution_nodes_session
    ON workflow_execution_nodes (session_id)
    WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_session_projection_public_list
    ON session_projection (
        workspace_identity, public_list_kind, public_sort_key_bits DESC, session_id
    );
CREATE INDEX IF NOT EXISTS idx_session_projection_public_node
    ON session_projection (
        workspace_identity, json_extract(public_summary, '$.node_id')
    )
    WHERE public_summary IS NOT NULL;
"#;

const NODE_EXECUTION_IDENTITY_V4: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_execution_nodes_node_execution
    ON workflow_execution_nodes (node_execution_id)
    WHERE node_execution_id IS NOT NULL;
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

const SESSION_PROJECTION_EVOLUTION_V3: &str = r#"
INSERT INTO session_projection (
    session_id, projection, revision, commit_id,
    workspace_identity, public_list_kind, public_sort_key_bits, public_summary
)
SELECT session_id, projection, revision, commit_id, NULL, NULL, NULL, NULL
FROM session_projection_v2;
DROP TABLE session_projection_v2;
"#;

fn evolve_session_projection_v3(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch("ALTER TABLE session_projection RENAME TO session_projection_v2;")?;
    connection.execute_batch(SESSION_PROJECTION_TABLE_V3)?;
    connection.execute_batch(SESSION_PROJECTION_EVOLUTION_V3)
}

fn create_store_metadata(
    connection: &Connection,
    table_name: &str,
    shutdown_plans_table: &str,
    schema_version: i64,
) -> Result<(), rusqlite::Error> {
    if !matches!(table_name, "store_metadata" | "store_metadata_v2")
        || !matches!(shutdown_plans_table, "shutdown_plans" | "shutdown_plans_v2")
        || !matches!(schema_version, 2..=5)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    connection.execute_batch(&format!(
        "CREATE TABLE {table_name} (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            schema_version INTEGER NOT NULL CHECK (schema_version = {schema_version}),
            installation_id TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
            cursor_hmac_key BLOB NOT NULL CHECK (length(cursor_hmac_key) = 32),
            operation_binding_hmac_key BLOB NOT NULL
                CHECK (length(operation_binding_hmac_key) = 32),
            process_instance_id TEXT NOT NULL,
            next_global_sequence INTEGER NOT NULL CHECK (next_global_sequence >= 1),
            health TEXT NOT NULL CHECK (health = 'ok'),
            current_shutdown_id TEXT,
            shutdown_pointer_revision INTEGER NOT NULL
                CHECK (shutdown_pointer_revision >= 0),
            FOREIGN KEY (current_shutdown_id)
                REFERENCES {shutdown_plans_table} (shutdown_id)
                DEFERRABLE INITIALLY DEFERRED
        );"
    ))
}

pub struct InitialStoreMetadata<'a> {
    pub installation_id: &'a str,
    pub cursor_hmac_key: &'a [u8; 32],
    pub operation_binding_hmac_key: &'a [u8; 32],
    pub process_instance_id: &'a str,
    pub created_at_ms: i64,
}

pub fn initialize_schema(
    connection: &Connection,
    metadata: &InitialStoreMetadata<'_>,
    fault: &FaultInjector,
) -> Result<(), rusqlite::Error> {
    connection.execute_batch("BEGIN IMMEDIATE;")?;
    if let Err(error) = (|| {
        connection.execute_batch(CURRENT_SCHEMA)?;
        connection.execute_batch(SESSION_PROJECTION_TABLE_V3)?;
        create_store_metadata(connection, "store_metadata", "shutdown_plans", 5)?;
        connection.execute_batch(WORKSPACE_QUERY_RECORDS_V3)?;
        connection.execute_batch(NODE_EXECUTION_IDENTITY_V4)?;
        connection.execute(
            "INSERT INTO store_metadata (
                id, schema_version, installation_id, created_at_ms,
                cursor_hmac_key, operation_binding_hmac_key, process_instance_id,
                next_global_sequence, health, current_shutdown_id,
                shutdown_pointer_revision
             ) VALUES (1, 5, ?1, ?2, ?3, ?4, ?5, 1, 'ok',
                       NULL, 0)",
            rusqlite::params![
                metadata.installation_id,
                metadata.created_at_ms,
                metadata.cursor_hmac_key.as_slice(),
                metadata.operation_binding_hmac_key.as_slice(),
                metadata.process_instance_id,
            ],
        )?;
        connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        Ok::<(), rusqlite::Error>(())
    })() {
        let _ = connection.execute_batch("ROLLBACK;");
        return Err(error);
    }
    if fault.take_initial_create_fault(InitialCreateFaultPoint::BeforeInitializationCommit) {
        #[cfg(test)]
        fault.crash_initial_create_process_if_armed(
            InitialCreateFaultPoint::BeforeInitializationCommit,
        );
        let _ = connection.execute_batch("ROLLBACK;");
        return Err(rusqlite::Error::InvalidQuery);
    }
    connection.execute_batch("COMMIT;")?;
    if fault.take_initial_create_fault(InitialCreateFaultPoint::AfterInitializationCommitReplyLoss)
    {
        #[cfg(test)]
        fault.crash_initial_create_process_if_armed(
            InitialCreateFaultPoint::AfterInitializationCommitReplyLoss,
        );
        return Err(rusqlite::Error::InvalidQuery);
    }
    finish_schema_admission(connection)
}

#[derive(Debug, Clone, Copy)]
enum SupportedMetadataVersion {
    V2,
    V3,
    V4,
}

fn carry_forward_store_metadata_v5(
    connection: &Connection,
    source_version: SupportedMetadataVersion,
) -> Result<(), rusqlite::Error> {
    let source_table = match source_version {
        SupportedMetadataVersion::V2 => "store_metadata_v2",
        SupportedMetadataVersion::V3 => "store_metadata_v3",
        SupportedMetadataVersion::V4 => "store_metadata_v4",
    };
    connection.execute_batch(&format!(
        "ALTER TABLE store_metadata RENAME TO {source_table};"
    ))?;
    create_store_metadata(connection, "store_metadata", "shutdown_plans", 5)?;
    connection.execute_batch(&format!(
        "INSERT INTO store_metadata (
             id, schema_version, installation_id, created_at_ms,
             cursor_hmac_key, operation_binding_hmac_key,
             process_instance_id, next_global_sequence, health,
             current_shutdown_id, shutdown_pointer_revision
         )
         SELECT id, 5, installation_id, created_at_ms,
                cursor_hmac_key, operation_binding_hmac_key,
                process_instance_id, next_global_sequence, health,
                current_shutdown_id, shutdown_pointer_revision
         FROM {source_table};
         DROP TABLE {source_table};"
    ))
}

fn evolve_schema_transaction(
    connection: &Connection,
    fault: &FaultInjector,
    evolution: impl FnOnce(&Connection) -> Result<(), rusqlite::Error>,
) -> Result<bool, rusqlite::Error> {
    if fault.take_schema_fail_before_begin() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    connection.execute_batch("PRAGMA foreign_keys = OFF; BEGIN IMMEDIATE;")?;
    let result = evolution(connection).and_then(|()| {
        connection.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        if fault.take_schema_fail_before_commit() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(())
    });
    if let Err(error) = result {
        let _ = connection.execute_batch("ROLLBACK; PRAGMA foreign_keys = ON;");
        return Err(error);
    }
    connection.execute_batch("COMMIT; PRAGMA foreign_keys = ON;")?;
    if fault.take_schema_commit_reply_loss() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    if fault.take_schema_fail_before_readback() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    validate_current_schema(connection)?;
    finish_schema_admission(connection)?;
    Ok(true)
}

pub fn evolve_schema(
    connection: &Connection,
    fault: &FaultInjector,
) -> Result<bool, rusqlite::Error> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let metadata_columns = table_columns(connection, "store_metadata")?;

    if application_id == i64::from(APPLICATION_ID)
        && user_version == CURRENT_SCHEMA_VERSION
        && metadata_columns
            .iter()
            .any(|column| column == "installation_id")
        && !metadata_columns.iter().any(|column| column == "store_id")
    {
        validate_current_schema(connection)?;
        finish_schema_admission(connection)?;
        return Ok(false);
    }

    let is_supported_v4 = application_id == i64::from(APPLICATION_ID)
        && user_version == 4
        && metadata_columns
            .iter()
            .any(|column| column == "installation_id")
        && !metadata_columns.iter().any(|column| column == "store_id");
    if is_supported_v4 {
        return evolve_schema_transaction(connection, fault, |connection| {
            carry_forward_store_metadata_v5(connection, SupportedMetadataVersion::V4)?;
            drop_retired_schema_v5(connection)?;
            Ok(())
        });
    }

    let is_supported_v3 = application_id == i64::from(APPLICATION_ID)
        && user_version == 3
        && metadata_columns
            .iter()
            .any(|column| column == "installation_id")
        && !metadata_columns.iter().any(|column| column == "store_id");
    if is_supported_v3 {
        return evolve_schema_transaction(connection, fault, |connection| {
            carry_forward_store_metadata_v5(connection, SupportedMetadataVersion::V3)?;
            connection.execute_batch(NODE_EXECUTION_IDENTITY_V4)?;
            drop_retired_schema_v5(connection)?;
            Ok(())
        });
    }

    let is_supported_v2 = application_id == i64::from(APPLICATION_ID)
        && user_version == 2
        && metadata_columns
            .iter()
            .any(|column| column == "installation_id")
        && !metadata_columns.iter().any(|column| column == "store_id");
    if is_supported_v2 {
        return evolve_schema_transaction(connection, fault, |connection| {
            carry_forward_store_metadata_v5(connection, SupportedMetadataVersion::V2)?;
            evolve_session_projection_v3(connection)?;
            connection.execute_batch(WORKSPACE_QUERY_RECORDS_V3)?;
            super::workspace_query_migration::rebuild_workspace_query_records(connection)?;
            connection.execute_batch(NODE_EXECUTION_IDENTITY_V4)?;
            drop_retired_schema_v5(connection)?;
            Ok(())
        });
    }

    let is_supported_v1 = metadata_columns.iter().any(|column| column == "store_id")
        && metadata_columns
            .iter()
            .any(|column| column == "generation_id")
        && metadata_columns.iter().any(|column| column == "boot_id");
    if !is_supported_v1 {
        return Err(rusqlite::Error::InvalidQuery);
    }

    evolve_schema_transaction(connection, fault, |connection| {
        // In v1, `generation_id` was the authority embedded in logical
        // idempotency keys, operation lookup keys, binding-HMAC preimages,
        // and caller-attempt seal contexts.
        // `store_id` identified the old physical cutover authority and could
        // legitimately differ. Adopt the former and converge every scoped row
        // to it so replay semantics do not split during v1 -> v2 evolution.
        connection.execute_batch(
            "DROP TABLE IF EXISTS migration_quit_flights;
             DROP TABLE IF EXISTS legacy_raw_record_chunks;
             DROP TABLE IF EXISTS legacy_raw_records;
             DROP TABLE IF EXISTS legacy_source_inventory;
             DROP TABLE IF EXISTS local_store_migrations;
             DROP TABLE IF EXISTS shutdown_compact_archives;
             DROP INDEX IF EXISTS idx_legacy_raw_records_source_path;
             DROP INDEX IF EXISTS idx_legacy_raw_record_chunks_source;
             ALTER TABLE operation_bindings RENAME COLUMN generation_id TO installation_id;
             ALTER TABLE caller_attempts RENAME COLUMN generation_id TO installation_id;
             UPDATE operation_bindings
                SET installation_id = (
                    SELECT generation_id FROM store_metadata WHERE id = 1
                );
             UPDATE caller_attempts
                SET installation_id = (
                    SELECT generation_id FROM store_metadata WHERE id = 1
                );
             DROP INDEX IF EXISTS idx_caller_attempts_scope;
             DROP INDEX IF EXISTS idx_caller_attempts_pending_kind;
             DROP INDEX IF EXISTS idx_operation_bindings_operation;
             CREATE TABLE logical_commits_v2 (
                 commit_id TEXT PRIMARY KEY,
                 installation_id TEXT NOT NULL,
                 operation_kind TEXT NOT NULL
                     CHECK (operation_kind IN (
                         'send', 'permission_response', 'stop', 'session_lifecycle',
                         'application_quit', 'recovery', 'user_mutation',
                         'shutdown_target', 'operation_progress', 'projection',
                         'workflow'
                     )),
                 idempotency_key TEXT NOT NULL,
                 payload_hash BLOB NOT NULL CHECK (length(payload_hash) = 32),
                 state TEXT NOT NULL CHECK (state IN ('preparing', 'sealed')),
                 first_global_sequence INTEGER
                     CHECK (first_global_sequence IS NULL OR first_global_sequence >= 1),
                 last_global_sequence INTEGER
                     CHECK (last_global_sequence IS NULL OR last_global_sequence >= 1),
                 event_count INTEGER NOT NULL CHECK (event_count >= 0),
                 mutation_count INTEGER NOT NULL CHECK (mutation_count >= 0),
                 stream_heads_json TEXT NOT NULL,
                 result_hash BLOB CHECK (result_hash IS NULL OR length(result_hash) = 32),
                 committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms >= 0),
                 UNIQUE (installation_id, operation_kind, idempotency_key),
                 CHECK ((first_global_sequence IS NULL) = (last_global_sequence IS NULL))
             );
             INSERT INTO logical_commits_v2 (
                 commit_id, installation_id, operation_kind, idempotency_key,
                 payload_hash, state, first_global_sequence, last_global_sequence,
                 event_count, mutation_count, stream_heads_json, result_hash,
                 committed_at_ms
             )
             SELECT commit_id,
                    (SELECT generation_id FROM store_metadata WHERE id = 1),
                    CASE operation_kind
                        WHEN 'migration' THEN 'projection'
                        ELSE operation_kind
                    END,
                    idempotency_key, payload_hash, state, first_global_sequence,
                    last_global_sequence, event_count, mutation_count,
                    stream_heads_json, result_hash, committed_at_ms
             FROM logical_commits;
             CREATE TABLE shutdown_plans_v2 (
                 shutdown_id TEXT PRIMARY KEY,
                 phase TEXT NOT NULL CHECK (phase IN (
                     'prepared', 'activated', 'quiescing',
                     'completed', 'failed', 'cancelled', 'reconciliation_required'
                 )),
                 summary TEXT NOT NULL,
                 details_state TEXT NOT NULL CHECK (details_state IN ('available', 'compacted')),
                 revision INTEGER NOT NULL CHECK (revision >= 0),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
             );
             INSERT INTO shutdown_plans_v2 (
                 shutdown_id, phase, summary, details_state, revision, commit_id
             )
             SELECT plan_id,
                    CASE phase WHEN 'preparing' THEN 'prepared' ELSE phase END,
                    summary, details_state, revision, commit_id
             FROM shutdown_plans;
             CREATE TABLE shutdown_targets_v2 (
                 shutdown_id TEXT NOT NULL,
                 ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                 detail TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK (revision >= 0),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (shutdown_id, ordinal),
                 FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans_v2 (shutdown_id)
             );
             INSERT INTO shutdown_targets_v2 (
                 shutdown_id, ordinal, detail, revision, commit_id
             )
             SELECT plan_id, ordinal, detail, revision, commit_id
             FROM shutdown_targets;
             CREATE TABLE shutdown_recovery_snapshots_v2 (
                 shutdown_id TEXT NOT NULL,
                 partition TEXT NOT NULL
                     CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
                 ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                 detail TEXT NOT NULL,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (shutdown_id, ordinal),
                 FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans_v2 (shutdown_id)
             );
             INSERT INTO shutdown_recovery_snapshots_v2 (
                 shutdown_id, partition, ordinal, detail, commit_id
             )
             SELECT plan_id, partition, ordinal, detail, commit_id
             FROM shutdown_recovery_snapshots;
             CREATE TABLE pending_obligations_v2 (
                 ordered_key TEXT PRIMARY KEY,
                 obligation_id TEXT NOT NULL UNIQUE REFERENCES obligations (obligation_id),
                 owner TEXT NOT NULL,
                 partition TEXT NOT NULL
                     CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
                 shutdown_id TEXT,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
             );
             INSERT INTO pending_obligations_v2 (
                 ordered_key, obligation_id, owner, partition, shutdown_id, commit_id
             )
             SELECT ordered_key, obligation_id, owner, partition, shutdown_plan_id, commit_id
             FROM pending_obligations;
             ALTER TABLE store_metadata RENAME TO store_metadata_v1;
             DROP TABLE pending_obligations;
             DROP TABLE shutdown_recovery_snapshots;
             DROP TABLE shutdown_targets;
             DROP TABLE shutdown_plans;
             DROP TABLE logical_commits;
             ALTER TABLE logical_commits_v2 RENAME TO logical_commits;
             ALTER TABLE shutdown_plans_v2 RENAME TO shutdown_plans;
             ALTER TABLE shutdown_targets_v2 RENAME TO shutdown_targets;
             ALTER TABLE shutdown_recovery_snapshots_v2 RENAME TO shutdown_recovery_snapshots;
             ALTER TABLE pending_obligations_v2 RENAME TO pending_obligations;
             UPDATE operation_records
                SET receipt = json_set(
                    json_remove(receipt, '$.plan_id', '$.epoch'),
                    '$.shutdown_id', operation_id
                )
              WHERE kind = 'application_quit'
                AND json_extract(receipt, '$.schema') = 'application_quit_receipt_v1';
             UPDATE operation_records
                SET latest_status = json_set(
                    json_remove(latest_status, '$.state.plan_id', '$.state.epoch'),
                    '$.state.shutdown_id', operation_id
                )
              WHERE kind = 'application_quit'
                AND json_extract(latest_status, '$.state.type') = 'outcome_unknown';
             UPDATE caller_attempts
                SET sealed_command = X'', resolution = 'cleared',
                    revision = CASE
                        WHEN revision < 9223372036854775807 THEN revision + 1
                        ELSE revision
                    END
              WHERE length(sealed_command) > 0
                AND substr(sealed_command, 1, 5) <> X'524C534131';
             CREATE INDEX idx_caller_attempts_scope
                 ON caller_attempts (principal, installation_id, scope_id, kind, caller_request_id);
             CREATE INDEX idx_caller_attempts_pending_kind
                 ON caller_attempts (installation_id, kind, resolution, principal, caller_request_id);
             CREATE INDEX idx_operation_bindings_operation
                 ON operation_bindings (installation_id, kind, operation_id, principal, caller_request_id);
             CREATE INDEX idx_pending_obligations_partition
                 ON pending_obligations (partition, ordered_key);
             CREATE INDEX idx_pending_obligations_owner
                 ON pending_obligations (owner, ordered_key);
             CREATE INDEX idx_pending_obligations_shutdown
                 ON pending_obligations (shutdown_id, ordered_key);
             CREATE INDEX idx_shutdown_plans_details_state
                 ON shutdown_plans (details_state);
             PRAGMA application_id = 0x524C5348;",
        )?;
        create_store_metadata(connection, "store_metadata", "shutdown_plans", 5)?;
        connection.execute_batch(
            "INSERT INTO store_metadata (
                 id, schema_version, installation_id, created_at_ms,
                 cursor_hmac_key, operation_binding_hmac_key, process_instance_id,
                 next_global_sequence, health, current_shutdown_id,
                 shutdown_pointer_revision
             )
             SELECT id, 5, generation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key, boot_id,
                    next_global_sequence, 'ok', current_shutdown_plan_id,
                    shutdown_pointer_revision
             FROM store_metadata_v1;
             DROP TABLE store_metadata_v1;",
        )?;
        evolve_session_projection_v3(connection)?;
        connection.execute_batch(WORKSPACE_QUERY_RECORDS_V3)?;
        super::workspace_query_migration::rebuild_workspace_query_records(connection)?;
        connection.execute_batch(NODE_EXECUTION_IDENTITY_V4)?;
        drop_retired_schema_v5(connection)?;
        Ok(())
    })
}

fn drop_retired_schema_v5(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "DROP INDEX IF EXISTS idx_message_projection_ordinal;
         DROP TABLE IF EXISTS message_projection;
         DROP TABLE IF EXISTS terminal_records;
         DROP TABLE IF EXISTS stop_resolutions;",
    )
}

fn finish_schema_admission(connection: &Connection) -> Result<(), rusqlite::Error> {
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
    let metadata: (i64, String, Vec<u8>, Vec<u8>, String) = connection.query_row(
        "SELECT schema_version, installation_id, cursor_hmac_key,
                operation_binding_hmac_key, process_instance_id
         FROM store_metadata WHERE id = 1",
        [],
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
    if metadata.0 != CURRENT_SCHEMA_VERSION
        || uuid::Uuid::parse_str(&metadata.1).is_err()
        || metadata.2.len() != 32
        || metadata.3.len() != 32
        || uuid::Uuid::parse_str(&metadata.4).is_err()
    {
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
            "cursor_hmac_key",
            "operation_binding_hmac_key",
            "process_instance_id",
            "next_global_sequence",
            "health",
            "current_shutdown_id",
            "shutdown_pointer_revision",
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
        "operation_bindings",
        &[
            "principal",
            "installation_id",
            "kind",
            "caller_request_id",
            "scope_id",
            "operation_id",
            "binding_hmac",
            "commit_id",
        ],
    )?;
    require_exact_columns(
        connection,
        "caller_attempts",
        &[
            "principal",
            "installation_id",
            "kind",
            "caller_request_id",
            "scope_id",
            "command_hash",
            "sealed_command",
            "resolution",
            "revision",
            "commit_id",
        ],
    )?;
    require_exact_columns(
        connection,
        "pending_obligations",
        &[
            "ordered_key",
            "obligation_id",
            "owner",
            "partition",
            "shutdown_id",
            "commit_id",
        ],
    )?;
    require_exact_columns(
        connection,
        "shutdown_plans",
        &[
            "shutdown_id",
            "phase",
            "summary",
            "details_state",
            "revision",
            "commit_id",
        ],
    )?;
    require_exact_columns(
        connection,
        "shutdown_targets",
        &["shutdown_id", "ordinal", "detail", "revision", "commit_id"],
    )?;
    require_exact_columns(
        connection,
        "shutdown_recovery_snapshots",
        &["shutdown_id", "partition", "ordinal", "detail", "commit_id"],
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
        "workflow_executions",
        &[
            "execution_id",
            "workspace_identity",
            "status",
            "list_kind",
            "sort_at_bits",
            "record_schema",
            "record",
            "source_revision",
            "commit_id",
        ],
    )?;
    require_exact_columns(
        connection,
        "workflow_execution_nodes",
        &[
            "execution_id",
            "node_id",
            "parent_id",
            "sibling_order",
            "session_id",
            "node_execution_id",
            "record_schema",
            "tree_record",
            "detail_record",
            "source_revision",
            "commit_id",
        ],
    )?;
    for index in [
        "idx_pending_obligations_partition",
        "idx_pending_obligations_owner",
        "idx_pending_obligations_shutdown",
        "idx_shutdown_plans_details_state",
        "idx_caller_attempts_scope",
        "idx_caller_attempts_pending_kind",
        "idx_operation_bindings_operation",
        "idx_workflow_executions_workspace_list",
        "idx_workflow_executions_global_list",
        "idx_workflow_execution_nodes_node",
        "idx_workflow_execution_nodes_occurrence",
        "idx_workflow_execution_nodes_node_execution",
        "idx_workflow_execution_nodes_session",
        "idx_session_projection_public_list",
        "idx_session_projection_public_node",
    ] {
        require_index(connection, index)?;
    }
    for table in ["message_projection", "terminal_records", "stop_resolutions"] {
        require_schema_object_absent(connection, "table", table)?;
    }
    require_schema_object_absent(connection, "index", "idx_message_projection_ordinal")?;
    for table in ["logical_commits", "operation_bindings", "caller_attempts"] {
        let divergent_identity_count: i64 = connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM {table}
                 WHERE installation_id <> (
                     SELECT installation_id FROM store_metadata WHERE id = 1
                 )"
            ),
            [],
            |row| row.get(0),
        )?;
        if divergent_identity_count != 0 {
            return Err(rusqlite::Error::InvalidQuery);
        }
    }
    require_foreign_key_integrity(connection)?;
    let integrity: String =
        connection.pragma_query_value(None, "integrity_check", |row| row.get(0))?;
    if integrity != "ok" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

pub fn validate_supported_schema_v1(connection: &Connection) -> Result<(), rusqlite::Error> {
    let application_id: i64 =
        connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
    let user_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (application_id != 0 && application_id != i64::from(APPLICATION_ID))
        || (user_version != 0 && user_version != 1)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let metadata_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM store_metadata", [], |row| row.get(0))?;
    if metadata_count != 1 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let metadata: (i64, String, String, Vec<u8>, Vec<u8>, String) = connection.query_row(
        "SELECT schema_version, store_id, generation_id, cursor_hmac_key,
                operation_binding_hmac_key, boot_id
         FROM store_metadata WHERE id = 1",
        [],
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
    if metadata.0 != 1
        || uuid::Uuid::parse_str(&metadata.1).is_err()
        || uuid::Uuid::parse_str(&metadata.2).is_err()
        || metadata.3.len() != 32
        || metadata.4.len() != 32
        || uuid::Uuid::parse_str(&metadata.5).is_err()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    for (table, required) in [
        (
            "logical_commits",
            &[
                "commit_id",
                "generation_id",
                "operation_kind",
                "idempotency_key",
            ][..],
        ),
        (
            "operation_bindings",
            &[
                "principal",
                "generation_id",
                "kind",
                "operation_id",
                "binding_hmac",
            ][..],
        ),
        (
            "caller_attempts",
            &[
                "principal",
                "generation_id",
                "kind",
                "caller_request_id",
                "scope_id",
            ][..],
        ),
        (
            "operation_records",
            &["kind", "operation_id", "receipt", "latest_status"][..],
        ),
        (
            "terminal_records",
            &["session_id", "turn_id", "terminal_identity", "result"][..],
        ),
        (
            "obligations",
            &["obligation_id", "record", "pending", "revision"][..],
        ),
        (
            "pending_obligations",
            &[
                "ordered_key",
                "obligation_id",
                "owner",
                "partition",
                "shutdown_plan_id",
            ][..],
        ),
        (
            "shutdown_plans",
            &[
                "plan_id",
                "epoch",
                "phase",
                "summary",
                "details_state",
                "revision",
            ][..],
        ),
        (
            "shutdown_targets",
            &["plan_id", "epoch", "ordinal", "detail", "revision"][..],
        ),
        (
            "shutdown_recovery_snapshots",
            &["plan_id", "epoch", "partition", "ordinal", "detail"][..],
        ),
    ] {
        require_columns(connection, table, required)?;
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

fn require_columns(
    connection: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), rusqlite::Error> {
    let columns = table_columns(connection, table)?;
    if expected
        .iter()
        .any(|expected| !columns.iter().any(|actual| actual == expected))
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn require_index(connection: &Connection, index: &str) -> Result<(), rusqlite::Error> {
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

fn require_schema_object_absent(
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

fn require_foreign_key_integrity(connection: &Connection) -> Result<(), rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(())
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> InitialStoreMetadata<'static> {
        InitialStoreMetadata {
            installation_id: "00000000-0000-4000-8000-000000000001",
            cursor_hmac_key: &[1; 32],
            operation_binding_hmac_key: &[2; 32],
            process_instance_id: "00000000-0000-4000-8000-000000000002",
            created_at_ms: 1,
        }
    }

    fn initialize(connection: &Connection) {
        initialize_schema(connection, &metadata(), &FaultInjector::new()).unwrap();
    }

    #[test]
    fn schema_v3_evolves_to_global_node_execution_identity() {
        let connection = Connection::open_in_memory().unwrap();
        initialize(&connection);
        connection
            .execute_batch(
                "BEGIN IMMEDIATE;
                 DROP INDEX idx_workflow_execution_nodes_node_execution;
                 ALTER TABLE store_metadata RENAME TO store_metadata_v4;",
            )
            .unwrap();
        create_store_metadata(&connection, "store_metadata", "shutdown_plans", 3).unwrap();
        connection
            .execute_batch(
                "INSERT INTO store_metadata (
                     id, schema_version, installation_id, created_at_ms,
                     cursor_hmac_key, operation_binding_hmac_key,
                     process_instance_id, next_global_sequence, health,
                     current_shutdown_id, shutdown_pointer_revision
                 )
                 SELECT id, 3, installation_id, created_at_ms,
                        cursor_hmac_key, operation_binding_hmac_key,
                        process_instance_id, next_global_sequence, health,
                        current_shutdown_id, shutdown_pointer_revision
                 FROM store_metadata_v4;
                 DROP TABLE store_metadata_v4;
                 PRAGMA user_version = 3;
                 COMMIT;",
            )
            .unwrap();

        assert!(evolve_schema(&connection, &FaultInjector::new()).unwrap());
        validate_current_schema(&connection).unwrap();
        require_index(&connection, "idx_workflow_execution_nodes_node_execution").unwrap();
    }

    #[test]
    fn node_execution_identity_is_unique_across_workflow_executions() {
        let connection = Connection::open_in_memory().unwrap();
        initialize(&connection);
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .unwrap();
        for execution_id in ["execution-1", "execution-2"] {
            connection
                .execute(
                    "INSERT INTO workflow_executions (
                         execution_id, workspace_identity, status, list_kind,
                         sort_at_bits, record_schema, record, source_revision, commit_id
                     ) VALUES (?1, ?2, 'running', 'active', 0,
                               'workflow_execution_record_v1', '{}', 0, 'commit')",
                    rusqlite::params![execution_id, format!("/{execution_id}")],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO workflow_execution_nodes (
                     execution_id, node_id, parent_id, sibling_order, session_id,
                     node_execution_id, record_schema, tree_record, detail_record,
                     source_revision, commit_id
                 ) VALUES ('execution-1', 'node-1', NULL, 0, NULL, 'node-execution-1',
                           'workflow_execution_node_record_v1', '{}', '{}', 0, 'commit')",
                [],
            )
            .unwrap();

        let duplicate = connection.execute(
            "INSERT INTO workflow_execution_nodes (
                 execution_id, node_id, parent_id, sibling_order, session_id,
                 node_execution_id, record_schema, tree_record, detail_record,
                 source_revision, commit_id
             ) VALUES ('execution-2', 'node-2', NULL, 0, NULL, 'node-execution-1',
                       'workflow_execution_node_record_v1', '{}', '{}', 0, 'commit')",
            [],
        );
        assert!(duplicate.is_err());
    }
}
