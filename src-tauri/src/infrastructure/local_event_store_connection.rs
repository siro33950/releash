//! SQLite connection configuration for the local event store.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

pub const MIN_SQLITE_VERSION_NUMBER: i32 = 3_051_003;

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

thread_local! { static BUSY_CONTEXT: std::cell::RefCell<crate::common::operation_context::OperationContext> = std::cell::RefCell::default(); }

pub struct ManagedConnection {
    connection: Connection,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
}

impl std::ops::Deref for ManagedConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}
impl std::ops::DerefMut for ManagedConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

pub(crate) fn configure_busy_handler(
    connection: Connection,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<ManagedConnection, rusqlite::Error> {
    unsafe extern "C" fn busy(
        data: *mut std::ffi::c_void,
        attempt: std::ffi::c_int,
    ) -> std::ffi::c_int {
        // SAFETY: ManagedConnection owns this Arc until after its SQLite connection closes.
        let limiter = unsafe { &*data.cast::<crate::common::retry::RetryLimiter>() };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if attempt == 0 {
                BUSY_CONTEXT.set(crate::common::operation_context::with_timeout(
                    Duration::from_secs(2),
                ));
            }
            BUSY_CONTEXT.with_borrow(|context| {
                limiter
                    .wait_sync(
                        context,
                        crate::common::retry::RetryBackoff::ITEM,
                        (attempt as u64).saturating_add(1),
                    )
                    .is_ok()
            })
        }))
        .unwrap_or(false) as std::ffi::c_int
    }
    // SAFETY: exclusive configuration of this connection; the pointer stays alive in ManagedConnection.
    let result = unsafe {
        rusqlite::ffi::sqlite3_busy_handler(
            connection.handle(),
            Some(busy),
            std::sync::Arc::as_ptr(&limiter).cast_mut().cast(),
        )
    };
    if result != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(result),
            None,
        ));
    }
    Ok(ManagedConnection {
        connection,
        limiter,
    })
}

impl ManagedConnection {
    pub(crate) fn retry_limiter(&self) -> std::sync::Arc<crate::common::retry::RetryLimiter> {
        self.limiter.clone()
    }
}

fn configure_common(connection: &Connection) -> Result<(), ConnectionError> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    Ok(())
}

fn configure_writer(connection: &Connection) -> Result<(), ConnectionError> {
    configure_common(connection)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

fn configure_reader(connection: &Connection) -> Result<(), ConnectionError> {
    // A reader must not participate in persistent journal configuration or
    // checkpoint ownership. Those effects belong to the single writer.
    configure_common(connection)
}

/// Open the single writer connection.
pub fn open_writer(
    path: &Path,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<ManagedConnection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let connection = configure_busy_handler(connection, limiter)?;
    configure_writer(&connection)?;
    Ok(connection)
}

/// Open an existing writer without allowing SQLite to create or replace it.
pub fn open_existing_writer(
    path: &Path,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<ManagedConnection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let connection = configure_busy_handler(connection, limiter)?;
    configure_writer(&connection)?;
    Ok(connection)
}

/// Open one reader-pool connection (read only).
pub fn open_reader(
    path: &Path,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<ManagedConnection, ConnectionError> {
    check_sqlite_version()?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let connection = configure_busy_handler(connection, limiter)?;
    configure_reader(&connection)?;
    Ok(connection)
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaInspectionError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid database file path: {0}")]
    InvalidPath(std::path::PathBuf),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub(crate) fn sqlite_sidecar_paths(database_path: &Path) -> [std::path::PathBuf; 2] {
    [
        std::path::PathBuf::from(format!("{}-wal", database_path.display())),
        std::path::PathBuf::from(format!("{}-shm", database_path.display())),
    ]
}

pub fn open_schema_inspection(
    path: &Path,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
    observe: impl Fn(crate::infrastructure::app_data_path::AppDataPathOperation, &Path),
) -> Result<ManagedConnection, SchemaInspectionError> {
    // Classification reads the fixed authority directly. SQLite's
    // `readonly_shm` URI mode sees committed WAL frames while mapping the
    // fixed SHM wal-index read-only, so a closed classification failure does
    // not claim a read-mark or change a sidecar byte. `immutable=1` is never
    // used when a non-empty WAL exists because it could ignore committed
    // frames. No create flag is permitted at this boundary.
    let [wal_path, shm_path] = sqlite_sidecar_paths(path);
    let mut wal_has_bytes = false;
    for sidecar in [wal_path.clone(), shm_path] {
        observe(
            crate::infrastructure::app_data_path::AppDataPathOperation::Metadata,
            &sidecar,
        );
        match std::fs::metadata(&sidecar) {
            Ok(metadata) => {
                observe(
                    crate::infrastructure::app_data_path::AppDataPathOperation::Open,
                    &sidecar,
                );
                observe(
                    crate::infrastructure::app_data_path::AppDataPathOperation::Read,
                    &sidecar,
                );
                if sidecar == wal_path {
                    wal_has_bytes = metadata.len() > 0;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(SchemaInspectionError::Io(error)),
        }
    }
    observe(
        crate::infrastructure::app_data_path::AppDataPathOperation::Open,
        path,
    );
    observe(
        crate::infrastructure::app_data_path::AppDataPathOperation::Read,
        path,
    );
    let mut uri = url::Url::from_file_path(path)
        .map_err(|()| SchemaInspectionError::InvalidPath(path.to_path_buf()))?;
    uri.query_pairs_mut().append_pair("mode", "ro");
    if wal_has_bytes {
        uri.query_pairs_mut().append_pair("readonly_shm", "1");
    } else {
        // With no committed WAL frame there is no sidecar state to include.
        // Immutable mode avoids asking a WAL-mode header for a missing SHM
        // while still opening this same fixed database path.
        uri.query_pairs_mut().append_pair("immutable", "1");
    }
    let connection = rusqlite::Connection::open_with_flags(
        uri.as_str(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    configure_busy_handler(connection, limiter).map_err(SchemaInspectionError::Sqlite)
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

#[cfg(test)]
#[path = "local_event_store_connection_test.rs"]
mod local_event_store_connection_tests;
