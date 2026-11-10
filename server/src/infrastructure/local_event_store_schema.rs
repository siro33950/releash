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
