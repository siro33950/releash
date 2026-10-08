use rusqlite::Connection;

pub(crate) fn create_store_metadata_v8(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE TABLE store_metadata (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            schema_version INTEGER NOT NULL CHECK (schema_version = 8),
            installation_id TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
            next_global_sequence INTEGER NOT NULL CHECK (next_global_sequence >= 1),
            health TEXT NOT NULL CHECK (health = 'ok')
        );",
    )
}

pub(crate) fn evolve_v8(
    connection: &Connection,
    copy_metadata: impl FnOnce(&Connection) -> Result<(), rusqlite::Error>,
) -> Result<(), rusqlite::Error> {
    connection.execute_batch("ALTER TABLE store_metadata RENAME TO store_metadata_v7;")?;
    create_store_metadata_v8(connection)?;
    copy_metadata(connection)?;
    connection.execute_batch(
        "DROP TABLE store_metadata_v7;
         DROP TABLE operation_bindings;
         DROP TABLE caller_attempts;
         DROP TABLE operation_records;
         DROP TABLE pending_obligations;
         DROP TABLE obligations;
         DROP TABLE recovery_action_attempts;
         DROP TABLE shutdown_targets;
         DROP TABLE shutdown_recovery_snapshots;
         DROP TABLE shutdown_plans;",
    )?;
    connection.pragma_update(None, "user_version", 8)
}

pub(crate) fn transaction(
    connection: &Connection,
    operation: impl FnOnce(&Connection) -> Result<(), rusqlite::Error>,
) -> Result<(), rusqlite::Error> {
    connection.execute_batch("BEGIN IMMEDIATE;")?;
    let result = operation(connection).and_then(|()| connection.execute_batch("COMMIT;"));
    if result.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    result
}

pub(crate) fn migration(
    connection: &Connection,
    operation: impl FnOnce(&Connection) -> Result<(), rusqlite::Error>,
) -> Result<(), rusqlite::Error> {
    connection.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let result = transaction(connection, operation);
    let restore = connection.execute_batch("PRAGMA foreign_keys = ON;");
    result.and(restore)
}
