//! Startup-only physical maintenance for the fixed SQLite authority.

use std::path::Path;

use rusqlite::Connection;

use super::fault::{FaultInjector, MaintenanceFaultPoint};
use super::layout::{StoreLayout, StorePathOperation};
use super::schema::validate_current_schema;
use crate::infrastructure::local_event_store_connection::{
    open_existing_writer, open_reader, set_owner_only_permissions, ConnectionError,
};
use crate::infrastructure::platform::file_replace;

const MINIMUM_RECLAIM_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreelistStats {
    page_count: u64,
    freelist_count: u64,
    page_size: u64,
}

#[derive(Debug)]
pub enum MaintenanceFailure {
    Sqlite(rusqlite::Error),
    Connection(ConnectionError),
    Io(std::io::Error),
    InvalidPragmaValue,
    InvalidPathEncoding,
    ArithmeticOverflow,
    Injected(MaintenanceFaultPoint),
}

impl std::fmt::Display for MaintenanceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::Connection(error) => write!(formatter, "connection error: {error}"),
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            Self::InvalidPragmaValue => formatter.write_str("invalid SQLite page statistic"),
            Self::InvalidPathEncoding => {
                formatter.write_str("vacuum database path is not valid UTF-8")
            }
            Self::ArithmeticOverflow => {
                formatter.write_str("SQLite page statistic arithmetic overflow")
            }
            Self::Injected(point) => write!(formatter, "injected fault at {point:?}"),
        }
    }
}

impl From<rusqlite::Error> for MaintenanceFailure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<ConnectionError> for MaintenanceFailure {
    fn from(error: ConnectionError) -> Self {
        Self::Connection(error)
    }
}

impl From<std::io::Error> for MaintenanceFailure {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub enum StartupMaintenanceError {
    Connection(ConnectionError),
}

impl std::fmt::Display for StartupMaintenanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connection(error) => write!(formatter, "connection error: {error}"),
        }
    }
}

impl std::error::Error for StartupMaintenanceError {}

pub fn run_startup_maintenance(
    layout: &StoreLayout,
    connection: crate::infrastructure::local_event_store_connection::ManagedConnection,
    fault: &FaultInjector,
) -> Result<
    crate::infrastructure::local_event_store_connection::ManagedConnection,
    StartupMaintenanceError,
> {
    let limiter = connection.retry_limiter();
    if let Err(error) = cleanup_vacuum_artifacts(layout) {
        log_failure("stale artifact cleanup", &error);
        return Ok(connection);
    }

    let stats = match read_freelist_stats(&connection) {
        Ok(stats) => stats,
        Err(error) => {
            log_failure("freelist inspection", &error);
            return Ok(connection);
        }
    };
    let should_reclaim = match should_reclaim(stats) {
        Ok(should_reclaim) => should_reclaim,
        Err(error) => {
            log_failure("freelist threshold calculation", &error);
            return Ok(connection);
        }
    };
    if !should_reclaim {
        log::debug!(
            "local event store startup maintenance skipped: page_count={}, freelist_count={}, page_size={}",
            stats.page_count,
            stats.freelist_count,
            stats.page_size
        );
        return Ok(connection);
    }

    if let Err(error) = prepare_vacuum_database(layout, &connection, fault, limiter.clone()) {
        log_failure("vacuum output preparation", &error);
        cleanup_after_failure(layout);
        return Ok(connection);
    }

    drop(connection);
    match replace_canonical_database(layout, fault) {
        Ok(()) => {}
        Err(CanonicalReplacementFailure::PreReplace(error)) => {
            log_failure("canonical database replacement", &error);
            cleanup_after_failure(layout);
            return reopen_canonical(layout, limiter.clone());
        }
        Err(CanonicalReplacementFailure::PostReplace(error)) => {
            log_failure(
                "canonical database replacement applied but directory durability failed",
                &error,
            );
            cleanup_after_failure(layout);
            return reopen_canonical(layout, limiter.clone());
        }
    }

    if let Err(error) = cleanup_vacuum_artifacts(layout) {
        log_failure("post-replacement artifact cleanup", &error);
    }
    let reopened = reopen_canonical(layout, limiter.clone())?;
    log::info!(
        "local event store startup maintenance reclaimed free pages: page_count={}, freelist_count={}, page_size={}",
        stats.page_count,
        stats.freelist_count,
        stats.page_size
    );
    Ok(reopened)
}

pub fn read_freelist_stats(connection: &Connection) -> Result<FreelistStats, MaintenanceFailure> {
    fn non_negative(value: i64) -> Result<u64, MaintenanceFailure> {
        u64::try_from(value).map_err(|_| MaintenanceFailure::InvalidPragmaValue)
    }

    let page_count = connection.pragma_query_value(None, "page_count", |row| row.get(0))?;
    let freelist_count = connection.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
    let page_size = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
    Ok(FreelistStats {
        page_count: non_negative(page_count)?,
        freelist_count: non_negative(freelist_count)?,
        page_size: non_negative(page_size)?,
    })
}

pub fn should_reclaim(stats: FreelistStats) -> Result<bool, MaintenanceFailure> {
    if stats.page_count == 0 {
        return Ok(false);
    }
    let ratio_numerator = stats
        .freelist_count
        .checked_mul(4)
        .ok_or(MaintenanceFailure::ArithmeticOverflow)?;
    let freelist_bytes = stats
        .freelist_count
        .checked_mul(stats.page_size)
        .ok_or(MaintenanceFailure::ArithmeticOverflow)?;
    Ok(ratio_numerator >= stats.page_count && freelist_bytes >= MINIMUM_RECLAIM_BYTES)
}

pub fn prepare_vacuum_database(
    layout: &StoreLayout,
    connection: &Connection,
    fault: &FaultInjector,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<(), MaintenanceFailure> {
    let vacuum_path = layout.vacuum_database_path();
    let vacuum_path_text = vacuum_path
        .to_str()
        .ok_or(MaintenanceFailure::InvalidPathEncoding)?;
    create_empty_vacuum_database(layout, &vacuum_path)?;
    inject(fault, MaintenanceFaultPoint::BeforeVacuumInto)?;
    layout.observe(StorePathOperation::Write, &vacuum_path);
    connection.execute("VACUUM INTO ?1", [vacuum_path_text])?;

    inject(fault, MaintenanceFaultPoint::BeforeOutputValidation)?;
    layout.observe(StorePathOperation::Open, &vacuum_path);
    layout.observe(StorePathOperation::Read, &vacuum_path);
    let output = open_reader(&vacuum_path, limiter)?;
    validate_current_schema(&output)?;
    drop(output);

    inject(fault, MaintenanceFaultPoint::BeforeOutputPermission)?;
    layout.observe(StorePathOperation::Metadata, &vacuum_path);
    set_owner_only_permissions(&vacuum_path)?;
    verify_owner_only_permissions(&vacuum_path)?;

    inject(fault, MaintenanceFaultPoint::BeforeOutputSync)?;
    layout.observe(StorePathOperation::Open, &vacuum_path);
    layout.observe(StorePathOperation::Sync, &vacuum_path);
    std::fs::File::open(&vacuum_path)?.sync_all()?;
    remove_paths(layout, &layout.vacuum_database_sidecar_paths())?;
    Ok(())
}

fn create_empty_vacuum_database(
    layout: &StoreLayout,
    vacuum_path: &Path,
) -> Result<(), MaintenanceFailure> {
    layout.observe(StorePathOperation::Write, vacuum_path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(vacuum_path)?;
    layout.observe(StorePathOperation::Metadata, vacuum_path);
    set_owner_only_permissions(vacuum_path)?;
    verify_owner_only_permissions(vacuum_path)?;
    drop(file);
    Ok(())
}

#[derive(Debug)]
pub enum CanonicalReplacementFailure {
    PreReplace(MaintenanceFailure),
    PostReplace(MaintenanceFailure),
}

pub fn replace_canonical_database(
    layout: &StoreLayout,
    fault: &FaultInjector,
) -> Result<(), CanonicalReplacementFailure> {
    inject(fault, MaintenanceFaultPoint::BeforeCanonicalSidecarCleanup)
        .map_err(CanonicalReplacementFailure::PreReplace)?;
    remove_paths(layout, &layout.database_sidecar_paths())
        .map_err(MaintenanceFailure::from)
        .map_err(CanonicalReplacementFailure::PreReplace)?;
    inject(fault, MaintenanceFaultPoint::BeforeReplace)
        .map_err(CanonicalReplacementFailure::PreReplace)?;

    let vacuum_path = layout.vacuum_database_path();
    let database_path = layout.database_path();
    layout.observe(StorePathOperation::Write, &vacuum_path);
    layout.observe(StorePathOperation::Write, &database_path);
    file_replace::replace_file(&vacuum_path, &database_path)
        .map_err(MaintenanceFailure::from)
        .map_err(CanonicalReplacementFailure::PreReplace)?;
    inject(fault, MaintenanceFaultPoint::AfterReplace)
        .map_err(CanonicalReplacementFailure::PostReplace)?;
    layout
        .sync_app_data_root()
        .map_err(MaintenanceFailure::from)
        .map_err(CanonicalReplacementFailure::PostReplace)?;
    Ok(())
}

fn reopen_canonical(
    layout: &StoreLayout,
    limiter: std::sync::Arc<crate::common::retry::RetryLimiter>,
) -> Result<
    crate::infrastructure::local_event_store_connection::ManagedConnection,
    StartupMaintenanceError,
> {
    let database_path = layout.database_path();
    layout.observe(StorePathOperation::Open, &database_path);
    layout.observe(StorePathOperation::Write, &database_path);
    open_existing_writer(&database_path, limiter).map_err(StartupMaintenanceError::Connection)
}

pub fn cleanup_vacuum_artifacts(layout: &StoreLayout) -> Result<(), std::io::Error> {
    let mut paths = Vec::with_capacity(3);
    paths.push(layout.vacuum_database_path());
    paths.extend(layout.vacuum_database_sidecar_paths());
    remove_paths(layout, &paths)
}

fn cleanup_after_failure(layout: &StoreLayout) {
    if let Err(error) = cleanup_vacuum_artifacts(layout) {
        log_failure("failure-exit artifact cleanup", &error);
    }
}

fn remove_paths(layout: &StoreLayout, paths: &[std::path::PathBuf]) -> Result<(), std::io::Error> {
    let mut first_error = None;
    for path in paths {
        layout.observe(StorePathOperation::Remove, path);
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub fn verify_owner_only_permissions(path: &Path) -> Result<(), std::io::Error> {
    let metadata = std::fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "vacuum database permissions are not owner-only",
            ));
        }
    }
    #[cfg(not(unix))]
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "vacuum database is not a regular file",
        ));
    }
    Ok(())
}

fn inject(fault: &FaultInjector, point: MaintenanceFaultPoint) -> Result<(), MaintenanceFailure> {
    if fault.take_maintenance_fault(point) {
        return Err(MaintenanceFailure::Injected(point));
    }
    Ok(())
}

fn log_failure(stage: &str, error: &dyn std::fmt::Display) {
    let correlation_id = uuid::Uuid::new_v4();
    log::warn!(
        "local event store startup maintenance failed at {stage} [{correlation_id}]: {error}"
    );
}

#[cfg(test)]
#[path = "maintenance_test.rs"]
mod maintenance_tests;
