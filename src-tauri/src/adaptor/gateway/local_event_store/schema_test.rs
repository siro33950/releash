use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;

use super::CURRENT_SCHEMA_VERSION;
use crate::adaptor::gateway::local_event_store::connection::open_existing_writer;
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
use crate::adaptor::gateway::local_event_store::store::{
    LocalEventStore, LocalEventStoreConfig, LocalEventStoreOpenError,
};

fn open_store(root: &Path) -> Arc<LocalEventStore> {
    LocalEventStore::open(LocalEventStoreConfig::production(root.to_path_buf()))
        .expect("file-backed local event store")
}

fn database_path(root: &Path) -> PathBuf {
    StoreLayout::new(root).database_path()
}

fn add_retired_schema_and_data(connection: &Connection, identity_column: &str) {
    connection
        .execute_batch(
            "CREATE TABLE message_projection (
                 session_id TEXT NOT NULL,
                 message_id TEXT NOT NULL,
                 message_ordinal INTEGER NOT NULL CHECK (message_ordinal > 0),
                 projection TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK (revision >= 0),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (session_id, message_id),
                 UNIQUE (session_id, message_ordinal)
             );
             CREATE TABLE terminal_records (
                 session_id TEXT NOT NULL,
                 turn_id TEXT NOT NULL,
                 terminal_identity TEXT NOT NULL,
                 result TEXT NOT NULL,
                 participant_digest BLOB NOT NULL CHECK (length(participant_digest) = 32),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
                 PRIMARY KEY (session_id, turn_id)
             );
             CREATE TABLE stop_resolutions (
                 stop_operation_id TEXT PRIMARY KEY,
                 resolution TEXT NOT NULL CHECK (resolution IN ('succeeded', 'superseded')),
                 detail TEXT NOT NULL,
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
             );
             CREATE UNIQUE INDEX idx_message_projection_ordinal
                 ON message_projection (session_id, message_ordinal);",
        )
        .unwrap();
    connection
        .execute(
            &format!(
                "INSERT INTO logical_commits (
                     commit_id, {identity_column}, operation_kind, idempotency_key,
                     payload_hash, state, first_global_sequence, last_global_sequence,
                     event_count, mutation_count, stream_heads_json, result_hash,
                     committed_at_ms
                 ) VALUES (
                     'retired-schema-commit',
                     (SELECT {identity_column} FROM store_metadata WHERE id = 1),
                     'projection', 'retired-schema', zeroblob(32), 'sealed',
                     NULL, NULL, 0, 0, '{{}}', NULL, 1
                 )"
            ),
            [],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO message_projection (
                 session_id, message_id, message_ordinal, projection, revision, commit_id
             ) VALUES ('retired-session', 'retired-message', 1, 'retired', 1,
                       'retired-schema-commit');
             INSERT INTO terminal_records (
                 session_id, turn_id, terminal_identity, result,
                 participant_digest, commit_id
             ) VALUES ('retired-session', 'retired-turn', 'retired-terminal', 'retired',
                       zeroblob(32), 'retired-schema-commit');
             INSERT INTO stop_resolutions (
                 stop_operation_id, resolution, detail, commit_id
             ) VALUES ('retired-stop', 'succeeded', 'retired', 'retired-schema-commit');",
        )
        .unwrap();
}

fn rewrite_metadata_version(connection: &Connection, version: i64) {
    connection
        .execute_batch(&format!(
            "PRAGMA foreign_keys = OFF;
             BEGIN IMMEDIATE;
             ALTER TABLE store_metadata RENAME TO store_metadata_v5;
             CREATE TABLE store_metadata (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 schema_version INTEGER NOT NULL CHECK (schema_version = {version}),
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
                     REFERENCES shutdown_plans (shutdown_id)
                     DEFERRABLE INITIALLY DEFERRED
             );
             INSERT INTO store_metadata (
                 id, schema_version, installation_id, created_at_ms,
                 cursor_hmac_key, operation_binding_hmac_key,
                 process_instance_id, next_global_sequence, health,
                 current_shutdown_id, shutdown_pointer_revision
             )
             SELECT id, {version}, installation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key,
                    process_instance_id, next_global_sequence, health,
                    current_shutdown_id, shutdown_pointer_revision
             FROM store_metadata_v5;
             DROP TABLE store_metadata_v5;
             PRAGMA user_version = {version};
             COMMIT;
             PRAGMA foreign_keys = ON;"
        ))
        .unwrap();
}

fn rewrite_as_supported_v1(connection: &Connection) {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF;
             BEGIN IMMEDIATE;
             DROP INDEX idx_caller_attempts_scope;
             DROP INDEX idx_caller_attempts_pending_kind;
             DROP INDEX idx_operation_bindings_operation;
             DROP INDEX idx_pending_obligations_partition;
             DROP INDEX idx_pending_obligations_owner;
             DROP INDEX idx_pending_obligations_shutdown;
             DROP INDEX idx_shutdown_plans_details_state;
             DROP INDEX idx_workflow_execution_nodes_node_execution;
             ALTER TABLE logical_commits
                 RENAME COLUMN installation_id TO generation_id;
             ALTER TABLE operation_bindings
                 RENAME COLUMN installation_id TO generation_id;
             ALTER TABLE caller_attempts
                 RENAME COLUMN installation_id TO generation_id;
             ALTER TABLE pending_obligations
                 RENAME COLUMN shutdown_id TO shutdown_plan_id;
             ALTER TABLE shutdown_plans RENAME COLUMN shutdown_id TO plan_id;
             ALTER TABLE shutdown_plans ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE shutdown_targets RENAME COLUMN shutdown_id TO plan_id;
             ALTER TABLE shutdown_targets ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE shutdown_recovery_snapshots
                 RENAME COLUMN shutdown_id TO plan_id;
             ALTER TABLE shutdown_recovery_snapshots
                 ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE store_metadata RENAME TO store_metadata_v5;
             CREATE TABLE store_metadata (
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
                 current_shutdown_plan_id TEXT,
                 shutdown_pointer_revision INTEGER NOT NULL
                     CHECK (shutdown_pointer_revision >= 0),
                 FOREIGN KEY (current_shutdown_plan_id)
                     REFERENCES shutdown_plans (plan_id)
                     DEFERRABLE INITIALLY DEFERRED
             );
             INSERT INTO store_metadata (
                 id, schema_version, store_id, generation_id, created_at_ms,
                 cursor_hmac_key, operation_binding_hmac_key, boot_id,
                 next_global_sequence, current_shutdown_plan_id,
                 shutdown_pointer_revision
             )
             SELECT id, 1, installation_id, installation_id, created_at_ms,
                    cursor_hmac_key, operation_binding_hmac_key,
                    process_instance_id, next_global_sequence,
                    current_shutdown_id, shutdown_pointer_revision
             FROM store_metadata_v5;
             DROP TABLE store_metadata_v5;
             ALTER TABLE session_projection RENAME TO session_projection_v5;
             CREATE TABLE session_projection (
                 session_id TEXT PRIMARY KEY,
                 projection TEXT NOT NULL,
                 revision INTEGER NOT NULL CHECK (revision >= 0),
                 commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
             );
             INSERT INTO session_projection (
                 session_id, projection, revision, commit_id
             )
             SELECT session_id, projection, revision, commit_id
             FROM session_projection_v5;
             DROP TABLE session_projection_v5;
             DROP TABLE workflow_execution_nodes;
             DROP TABLE workflow_executions;
             PRAGMA user_version = 1;
             COMMIT;
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
}

fn create_supported_store(root: &Path, version: i64) {
    drop(open_store(root));
    let connection = open_existing_writer(&database_path(root)).unwrap();
    if version == 1 {
        rewrite_as_supported_v1(&connection);
        add_retired_schema_and_data(&connection, "generation_id");
    } else {
        if version <= 3 {
            connection
                .execute_batch("DROP INDEX idx_workflow_execution_nodes_node_execution;")
                .unwrap();
        }
        rewrite_metadata_version(&connection, version);
        add_retired_schema_and_data(&connection, "installation_id");
    }
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
}

fn assert_retired_schema_absent(connection: &Connection) {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name IN (
                 'message_projection', 'terminal_records', 'stop_resolutions',
                 'idx_message_projection_ordinal'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_SCHEMA_VERSION);
    let metadata_version: i64 = connection
        .query_row(
            "SELECT schema_version FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(metadata_version, CURRENT_SCHEMA_VERSION);
}

#[test]
fn test_schema_v5_新規作成と再起動で廃止schemaを作成しない() {
    let root = tempfile::TempDir::new().unwrap();

    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    assert_retired_schema_absent(&connection);
    drop(connection);

    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    assert_retired_schema_absent(&connection);
}

#[test]
fn test_schema_v5_supported_schema_v1からv4を開くと廃止schemaを削除する() {
    for version in 1..=4 {
        let root = tempfile::TempDir::new().unwrap();
        create_supported_store(root.path(), version);

        drop(open_store(root.path()));
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        assert_retired_schema_absent(&connection);
        let retained_commit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM logical_commits
                 WHERE commit_id = 'retired-schema-commit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_commit_count, 1, "supported schema v{version}");
        drop(connection);

        drop(open_store(root.path()));
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        assert_retired_schema_absent(&connection);
    }
}

#[test]
fn test_schema_v5_移行commit前の失敗ではv4と廃止dataを原子的に維持する() {
    let root = tempfile::TempDir::new().unwrap();
    create_supported_store(root.path(), 4);
    let fault = Arc::new(FaultInjector::new());
    fault.arm_schema_fail_before_commit();
    let mut config = LocalEventStoreConfig::production(root.path().to_path_buf());
    config.fault = fault;

    let error = match LocalEventStore::open(config) {
        Ok(_) => panic!("schema evolution fault must fail store open"),
        Err(error) => error,
    };
    assert_eq!(error, LocalEventStoreOpenError::SchemaEvolutionFailed);
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let retired_object_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name IN (
                 'message_projection', 'terminal_records', 'stop_resolutions',
                 'idx_message_projection_ordinal'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retired_object_count, 4);
    let retired_data_count: i64 = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM message_projection)
               + (SELECT COUNT(*) FROM terminal_records)
               + (SELECT COUNT(*) FROM stop_resolutions)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retired_data_count, 3);
}
