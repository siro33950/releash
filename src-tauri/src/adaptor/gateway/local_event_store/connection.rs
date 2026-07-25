//! SQLite connection configuration for the local event store.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::adaptor::gateway::local_event_store::schema::MIN_SQLITE_VERSION_NUMBER;

#[derive(Debug)]
pub enum ConnectionError {
    SqliteTooOld { version_number: i32 },
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SqliteTooOld { version_number } => write!(
                f,
                "bundled SQLite {version_number} is older than required {MIN_SQLITE_VERSION_NUMBER}"
            ),
            Self::Sqlite(inner) => write!(f, "sqlite error: {inner}"),
        }
    }
}

impl std::error::Error for ConnectionError {}

impl From<rusqlite::Error> for ConnectionError {
    fn from(inner: rusqlite::Error) -> Self {
        Self::Sqlite(inner)
    }
}

/// Startup check that the bundled SQLite satisfies the minimum version.
pub fn check_sqlite_version() -> Result<(), ConnectionError> {
    let version_number = rusqlite::version_number();
    if version_number < MIN_SQLITE_VERSION_NUMBER {
        return Err(ConnectionError::SqliteTooOld { version_number });
    }
    Ok(())
}

fn configure(connection: &Connection) -> Result<(), ConnectionError> {
    connection.busy_timeout(Duration::from_secs(2))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    Ok(())
}

/// Open the single writer connection.
pub fn open_writer(path: &Path) -> Result<Connection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&connection)?;
    Ok(connection)
}

/// Open an existing writer without allowing SQLite to create or replace it.
pub fn open_existing_writer(path: &Path) -> Result<Connection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&connection)?;
    Ok(connection)
}

/// Open one reader-pool connection (read only).
pub fn open_reader(path: &Path) -> Result<Connection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    configure(&connection)?;
    Ok(connection)
}

/// Restrict a store file / directory to the owning user.
pub fn set_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(if metadata.is_dir() { 0o700 } else { 0o600 });
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
