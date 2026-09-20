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

pub(super) fn restore_v7_schema(connection: &Connection) {
    connection
        .execute_batch(include_str!("schema_v7_test.sql"))
        .unwrap();
    connection.execute_batch("ALTER TABLE store_metadata ADD COLUMN current_shutdown_id TEXT; ALTER TABLE store_metadata ADD COLUMN shutdown_pointer_revision INTEGER NOT NULL DEFAULT 0;").unwrap();
    connection.execute_batch("ALTER TABLE store_metadata ADD COLUMN cursor_hmac_key BLOB NOT NULL DEFAULT X'0000000000000000000000000000000000000000000000000000000000000000';
        ALTER TABLE store_metadata ADD COLUMN operation_binding_hmac_key BLOB NOT NULL DEFAULT X'0000000000000000000000000000000000000000000000000000000000000000';
        ALTER TABLE store_metadata ADD COLUMN process_instance_id TEXT NOT NULL DEFAULT '00000000-0000-4000-8000-000000000002';").unwrap();
    rewrite_metadata_version(connection, 7);
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
             PRAGMA user_version = 1;
             COMMIT;
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
}

fn create_supported_store(root: &Path, version: i64) {
    drop(open_store(root));
    let connection = open_existing_writer(&database_path(root)).unwrap();
    restore_v7_schema(&connection);
    if version == 1 {
        rewrite_as_supported_v1(&connection);
        add_retired_schema_and_data(&connection, "generation_id");
    } else {
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
fn test_schema_v7_v6からevent_type索引を追加してversionを更新する() {
    let root = tempfile::TempDir::new().unwrap();
    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    connection
        .execute_batch("DROP INDEX idx_node_events_event_type;")
        .unwrap();
    restore_v7_schema(&connection);
    rewrite_metadata_version(&connection, 6);
    drop(connection);

    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();

    assert_retired_schema_absent(&connection);
    let index_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE type = 'index' AND name = 'idx_node_events_event_type'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_count, 1);
}

#[test]
fn test_schema_v5_supported_schema_v1からv4を開くと廃止schemaを削除する() {
    for version in 1..=4 {
        let root = tempfile::TempDir::new().unwrap();
        create_supported_store(root.path(), version);

        drop(open_store(root.path()));
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        assert_retired_schema_absent(&connection);
        // v6: どのテーブルからも参照されなくなった commit 台帳（孤児）は
        // 廃止データと一緒に掃除される（D3）。
        let retained_commit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM logical_commits
                 WHERE commit_id = 'retired-schema-commit'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_commit_count, 0, "supported schema v{version}");
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

#[tokio::test]
async fn test_schema_v8_未完了の終了記録があってもsession作成と読み書きができる() {
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::provider_lifecycle::ProviderKind;
    use crate::domain::workspace_tree::WorkspaceIdentity;

    for phase in [
        "prepared",
        "activated",
        "quiescing",
        "reconciliation_required",
        "completed",
        "failed",
        "cancelled",
    ] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let store = open_store(directory.path());
        let session = |id| {
            AgentSession::create(
                id,
                WorkspaceIdentity::new("/repo"),
                "/repo",
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(id).unwrap(),
            )
            .unwrap()
        };
        LocalAgentSessionRepository::new(store.clone())
            .create(session("existing"), "create-existing")
            .await
            .unwrap();
        drop(store);
        let connection = open_existing_writer(&database_path(directory.path())).unwrap();
        restore_v7_schema(&connection);
        let commit_id: String = connection
            .query_row("SELECT commit_id FROM logical_commits LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        connection
            .execute(
                "INSERT INTO shutdown_plans VALUES ('old-quit', ?1, '{}', 'available', 0, ?2)",
                rusqlite::params![phase, commit_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO shutdown_targets VALUES ('old-quit', 0, '{}', 0, ?1)",
                [&commit_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO shutdown_recovery_snapshots VALUES ('old-quit', 'owner', 0, '{}', ?1)",
                [&commit_id],
            )
            .unwrap();
        connection.execute("INSERT INTO operation_bindings VALUES ('local', (SELECT installation_id FROM store_metadata), 'application_quit', 'request', 'application', 'old-quit', zeroblob(32), ?1)", [&commit_id]).unwrap();
        connection.execute("INSERT INTO caller_attempts VALUES ('local', (SELECT installation_id FROM store_metadata), 'application_quit', 'request', 'application', zeroblob(32), X'01', 'pending', 0, ?1)", [&commit_id]).unwrap();
        connection.execute("INSERT INTO operation_records VALUES ('application_quit', 'old-quit', '{}', '{}', 0, ?1)", [&commit_id]).unwrap();
        connection
            .execute(
                "INSERT INTO obligations VALUES ('effect', '{}', 1, 0, ?1)",
                [&commit_id],
            )
            .unwrap();
        connection.execute("INSERT INTO pending_obligations VALUES ('effect', 'effect', 'existing', 'owner', 'old-quit', ?1)", [&commit_id]).unwrap();
        connection.execute("INSERT INTO recovery_action_attempts VALUES ('retry', zeroblob(32), '{}', NULL, 0, ?1)", [&commit_id]).unwrap();
        connection.execute("UPDATE store_metadata SET current_shutdown_id = 'old-quit', shutdown_pointer_revision = 1", []).unwrap();
        drop(connection);
        // When
        let store = open_store(directory.path());
        let repository = LocalAgentSessionRepository::new(store.clone());
        let mut saved = repository.find("existing").await.unwrap().unwrap();
        saved
            .session_mut()
            .associate_provider_session("provider-existing", None)
            .unwrap();
        repository.save(saved, "open-existing").await.unwrap();
        repository
            .create(session("new"), "create-new")
            .await
            .unwrap();
        // Then
        assert!(repository.find("new").await.unwrap().is_some());
        let connection = open_existing_writer(&database_path(directory.path())).unwrap();
        super::validate_current_schema(&connection).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            8
        );
        for table in [
            "operation_bindings",
            "caller_attempts",
            "operation_records",
            "obligations",
            "pending_obligations",
            "recovery_action_attempts",
            "shutdown_plans",
            "shutdown_targets",
            "shutdown_recovery_snapshots",
        ] {
            super::require_schema_object_absent(&connection, "table", table).unwrap();
        }
        let columns = super::table_columns(&connection, "store_metadata").unwrap();
        for removed in [
            "current_shutdown_id",
            "shutdown_pointer_revision",
            "cursor_hmac_key",
            "operation_binding_hmac_key",
            "process_instance_id",
        ] {
            assert!(
                !columns.contains(&removed.to_string()),
                "{removed} must be removed"
            );
        }
    }
}

#[test]
fn test_schema_v8_新規storeと再起動で用途を失ったmetadataを持たない() {
    // Given
    let root = tempfile::tempdir().unwrap();
    // When / Then
    for _ in 0..2 {
        drop(open_store(root.path()));
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        assert_eq!(
            super::table_columns(&connection, "store_metadata").unwrap(),
            [
                "id",
                "schema_version",
                "installation_id",
                "created_at_ms",
                "next_global_sequence",
                "health",
            ]
        );
        super::validate_current_schema(&connection).unwrap();
    }
}

#[test]
fn test_schema_v8_移行失敗では旧metadataを残し再起動で必要な値だけ引き継ぐ() {
    // Given
    let root = tempfile::tempdir().unwrap();
    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    restore_v7_schema(&connection);
    let metadata = |connection: &Connection| -> (String, i64, i64, String) {
        connection.query_row("SELECT installation_id, created_at_ms, next_global_sequence, health FROM store_metadata", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        }).unwrap()
    };
    let before = metadata(&connection);
    drop(connection);
    let fault = Arc::new(FaultInjector::new());
    fault.arm_schema_fail_before_commit();
    let mut config = LocalEventStoreConfig::production(root.path().to_path_buf());
    config.fault = fault;
    // When
    assert!(matches!(
        LocalEventStore::open(config),
        Err(LocalEventStoreOpenError::SchemaEvolutionFailed)
    ));
    // Then
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(metadata(&connection), before);
    let columns = super::table_columns(&connection, "store_metadata").unwrap();
    for old in [
        "cursor_hmac_key",
        "operation_binding_hmac_key",
        "process_instance_id",
    ] {
        assert!(columns.contains(&old.to_string()));
    }
    drop(connection);
    drop(open_store(root.path()));
    let connection = open_existing_writer(&database_path(root.path())).unwrap();
    assert_eq!(metadata(&connection), before);
    super::validate_current_schema(&connection).unwrap();
}
