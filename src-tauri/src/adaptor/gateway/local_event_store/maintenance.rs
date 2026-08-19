//! Startup-only physical maintenance for the fixed SQLite authority.

use std::path::Path;

use rusqlite::Connection;

use super::connection::{
    open_existing_writer, open_reader, set_owner_only_permissions, ConnectionError,
};
use super::fault::{FaultInjector, MaintenanceFaultPoint};
use super::layout::{StoreLayout, StorePathOperation};
use super::schema::validate_current_schema;
use crate::infrastructure::platform::file_replace;

const MINIMUM_RECLAIM_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FreelistStats {
    page_count: u64,
    freelist_count: u64,
    page_size: u64,
}

#[derive(Debug)]
enum MaintenanceFailure {
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
    connection: Connection,
    fault: &FaultInjector,
) -> Result<Connection, StartupMaintenanceError> {
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

    if let Err(error) = prepare_vacuum_database(layout, &connection, fault) {
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
            return reopen_canonical(layout);
        }
        Err(CanonicalReplacementFailure::PostReplace(error)) => {
            log_failure(
                "canonical database replacement applied but directory durability failed",
                &error,
            );
            cleanup_after_failure(layout);
            return reopen_canonical(layout);
        }
    }

    if let Err(error) = cleanup_vacuum_artifacts(layout) {
        log_failure("post-replacement artifact cleanup", &error);
    }
    let reopened = reopen_canonical(layout)?;
    log::info!(
        "local event store startup maintenance reclaimed free pages: page_count={}, freelist_count={}, page_size={}",
        stats.page_count,
        stats.freelist_count,
        stats.page_size
    );
    Ok(reopened)
}

fn read_freelist_stats(connection: &Connection) -> Result<FreelistStats, MaintenanceFailure> {
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

fn should_reclaim(stats: FreelistStats) -> Result<bool, MaintenanceFailure> {
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

fn prepare_vacuum_database(
    layout: &StoreLayout,
    connection: &Connection,
    fault: &FaultInjector,
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
    let output = open_reader(&vacuum_path)?;
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
enum CanonicalReplacementFailure {
    PreReplace(MaintenanceFailure),
    PostReplace(MaintenanceFailure),
}

fn replace_canonical_database(
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

fn reopen_canonical(layout: &StoreLayout) -> Result<Connection, StartupMaintenanceError> {
    let database_path = layout.database_path();
    layout.observe(StorePathOperation::Open, &database_path);
    layout.observe(StorePathOperation::Write, &database_path);
    open_existing_writer(&database_path).map_err(StartupMaintenanceError::Connection)
}

fn cleanup_vacuum_artifacts(layout: &StoreLayout) -> Result<(), std::io::Error> {
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

fn verify_owner_only_permissions(path: &Path) -> Result<(), std::io::Error> {
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
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::adaptor::gateway::local_event_store::store::{
        LocalEventStore, LocalEventStoreConfig,
    };
    use crate::domain::local_event::{
        CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
        LocalEventTransactionRepository,
    };
    use crate::infrastructure::app_data_path::{AppDataPathObserver, AppDataPathOperation};

    #[derive(Default)]
    struct RecordingObserver {
        operations: Mutex<Vec<(AppDataPathOperation, PathBuf)>>,
    }

    impl AppDataPathObserver for RecordingObserver {
        fn observe(&self, operation: AppDataPathOperation, path: &Path) {
            self.operations
                .lock()
                .expect("maintenance observer")
                .push((operation, path.to_path_buf()));
        }
    }

    impl RecordingObserver {
        fn observed(&self, operation: AppDataPathOperation, path: &Path) -> bool {
            self.operations
                .lock()
                .expect("maintenance observer")
                .iter()
                .any(|observed| observed == &(operation, path.to_path_buf()))
        }
    }

    fn open_store(root: &Path) -> Arc<LocalEventStore> {
        LocalEventStore::open(LocalEventStoreConfig::production(root.to_path_buf()))
            .expect("file-backed local event store")
    }

    fn open_store_with_fault(
        root: &Path,
        fault: Arc<FaultInjector>,
        observer: Arc<dyn AppDataPathObserver>,
    ) -> Arc<LocalEventStore> {
        let mut config = LocalEventStoreConfig::production(root.to_path_buf());
        config.fault = fault;
        config.path_observer = observer;
        LocalEventStore::open(config).expect("file-backed local event store with maintenance fault")
    }

    fn database_path(root: &Path) -> PathBuf {
        StoreLayout::new(root).database_path()
    }

    fn create_fragmented_store(root: &Path) -> (u64, String) {
        let store = open_store(root);
        let installation_id = store.installation_id().to_string();
        drop(store);
        let path = database_path(root);
        let connection = open_existing_writer(&path).unwrap();
        connection
            .execute_batch("PRAGMA secure_delete = OFF;")
            .unwrap();
        connection
            .execute(
                "INSERT INTO logical_commits (
                     commit_id, installation_id, operation_kind, idempotency_key,
                     payload_hash, state, first_global_sequence, last_global_sequence,
                     event_count, mutation_count, stream_heads_json, result_hash,
                     committed_at_ms
                 ) VALUES (
                     'maintenance-preserved', ?1, 'projection', 'maintenance-preserved',
                     zeroblob(32), 'sealed', NULL, NULL, 0, 0, '{}', NULL, 1
                 )",
                [&installation_id],
            )
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO stream_heads (stream_id, head, updated_commit_id)
                     VALUES ('maintenance-stream', 1, 'maintenance-preserved');
                 INSERT INTO events (
                     global_sequence, event_id, commit_id, stream_id, stream_sequence,
                     event_type, payload_version, occurred_at, payload, payload_sha256
                 ) VALUES (
                     1, 'maintenance-event', 'maintenance-preserved', 'maintenance-stream', 1,
                     'maintenance.event', 1, '2026-01-01T00:00:00Z', X'01', zeroblob(32)
                 );
                 INSERT INTO operation_bindings (
                     principal, installation_id, kind, caller_request_id, scope_id,
                     operation_id, binding_hmac, commit_id
                 ) VALUES (
                     'maintenance-principal',
                     (SELECT installation_id FROM store_metadata WHERE id = 1),
                     'send', 'maintenance-request', 'maintenance-scope',
                     'maintenance-operation', zeroblob(32), 'maintenance-preserved'
                 );
                 INSERT INTO caller_attempts (
                     principal, installation_id, kind, caller_request_id, scope_id,
                     command_hash, sealed_command, resolution, revision, commit_id
                 ) VALUES (
                     'maintenance-principal',
                     (SELECT installation_id FROM store_metadata WHERE id = 1),
                     'send', 'maintenance-request', 'maintenance-scope',
                     zeroblob(32), X'01', 'accepted', 1, 'maintenance-preserved'
                 );
                 INSERT INTO operation_records (
                     kind, operation_id, receipt, latest_status, revision, commit_id
                 ) VALUES (
                     'send', 'maintenance-operation', '{}', '{}', 1,
                     'maintenance-preserved'
                 );
                 INSERT INTO session_projection (
                     session_id, projection, revision, commit_id
                 ) VALUES (
                     'maintenance-session', '{}', 1, 'maintenance-preserved'
                 );
                 INSERT INTO obligations (
                     obligation_id, record, pending, revision, commit_id
                 ) VALUES (
                     'maintenance-obligation', '{}', 1, 1, 'maintenance-preserved'
                 );
                 INSERT INTO pending_obligations (
                     ordered_key, obligation_id, owner, partition, shutdown_id, commit_id
                 ) VALUES (
                     'maintenance-ordered', 'maintenance-obligation', 'maintenance-owner',
                     'owner', NULL, 'maintenance-preserved'
                 );
                 INSERT INTO recovery_action_attempts (
                     action_id, binding_hash, attempt, completed, revision, commit_id
                 ) VALUES (
                     'maintenance-action', zeroblob(32), '{}', '{}', 1,
                     'maintenance-preserved'
                 );
                 INSERT INTO shutdown_plans (
                     shutdown_id, phase, summary, details_state, revision, commit_id
                 ) VALUES (
                     'maintenance-shutdown', 'prepared', 'maintenance', 'available', 1,
                     'maintenance-preserved'
                 );
                 UPDATE pending_obligations
                    SET shutdown_id = 'maintenance-shutdown'
                  WHERE ordered_key = 'maintenance-ordered';
                 INSERT INTO shutdown_targets (
                     shutdown_id, ordinal, detail, revision, commit_id
                 ) VALUES (
                     'maintenance-shutdown', 0, '{}', 1, 'maintenance-preserved'
                 );
                 INSERT INTO shutdown_recovery_snapshots (
                     shutdown_id, partition, ordinal, detail, commit_id
                 ) VALUES (
                     'maintenance-shutdown', 'owner', 0, '{}', 'maintenance-preserved'
                 );
                 INSERT INTO node_events (
                     tree_id, seq, node_execution_id, parent_id, node_name, kind,
                     attempt, event_type, detail, timestamp
                 ) VALUES (
                     'maintenance-execution', 1, 'maintenance-node-execution', NULL,
                     'main', 'session', 1, 'started', '{}', 1
                 );
                 UPDATE store_metadata
                    SET next_global_sequence = 2,
                        shutdown_pointer_revision = 1
                  WHERE id = 1;",
            )
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE startup_maintenance_free_space (payload BLOB NOT NULL);
                 INSERT INTO startup_maintenance_free_space (payload)
                     VALUES (zeroblob(71303168));
                 DROP TABLE startup_maintenance_free_space;
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
        let stats = read_freelist_stats(&connection).unwrap();
        assert!(
            should_reclaim(stats).unwrap(),
            "fixture must cross both thresholds"
        );
        drop(connection);
        (std::fs::metadata(path).unwrap().len(), installation_id)
    }

    #[derive(Debug, PartialEq)]
    struct StoreSnapshot {
        tables: Vec<(&'static str, Vec<Vec<rusqlite::types::Value>>)>,
        metadata: Vec<rusqlite::types::Value>,
    }

    fn snapshot_store(connection: &Connection) -> StoreSnapshot {
        const TABLES: [&str; 14] = [
            "logical_commits",
            "stream_heads",
            "events",
            "operation_bindings",
            "caller_attempts",
            "operation_records",
            "session_projection",
            "obligations",
            "pending_obligations",
            "recovery_action_attempts",
            "shutdown_plans",
            "shutdown_targets",
            "shutdown_recovery_snapshots",
            "node_events",
        ];

        let tables = TABLES
            .into_iter()
            .map(|table| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                    .unwrap();
                let column_count = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..column_count)
                            .map(|index| row.get(index))
                            .collect::<Result<Vec<rusqlite::types::Value>, _>>()
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                (table, rows)
            })
            .collect();
        let metadata = connection
            .query_row(
                "SELECT id, schema_version, installation_id, created_at_ms,
                        cursor_hmac_key, operation_binding_hmac_key,
                        next_global_sequence, health, current_shutdown_id,
                        shutdown_pointer_revision
                 FROM store_metadata WHERE id = 1",
                [],
                |row| {
                    (0..10)
                        .map(|index| row.get(index))
                        .collect::<Result<Vec<rusqlite::types::Value>, _>>()
                },
            )
            .unwrap();
        StoreSnapshot { tables, metadata }
    }

    fn snapshot_store_path(root: &Path) -> StoreSnapshot {
        let connection = open_existing_writer(&database_path(root)).unwrap();
        snapshot_store(&connection)
    }

    fn assert_preserved_content(root: &Path, installation_id: &str) {
        let connection = open_existing_writer(&database_path(root)).unwrap();
        let stored_installation_id: String = connection
            .query_row(
                "SELECT installation_id FROM store_metadata WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_installation_id, installation_id);
        let commit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM logical_commits
                 WHERE commit_id = 'maintenance-preserved'
                   AND idempotency_key = 'maintenance-preserved'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(commit_count, 1);
        super::super::schema::validate_current_schema(&connection).unwrap();
    }

    #[test]
    fn test_物理回収判定_二つの閾値をともに満たす場合だけ発火する() {
        let page_size = 4096;
        let minimum_pages = MINIMUM_RECLAIM_BYTES / page_size;

        assert!(should_reclaim(FreelistStats {
            page_count: minimum_pages * 4,
            freelist_count: minimum_pages,
            page_size,
        })
        .unwrap());
        assert!(!should_reclaim(FreelistStats {
            page_count: minimum_pages * 4 + 1,
            freelist_count: minimum_pages,
            page_size,
        })
        .unwrap());
        assert!(!should_reclaim(FreelistStats {
            page_count: (minimum_pages - 1) * 4,
            freelist_count: minimum_pages - 1,
            page_size,
        })
        .unwrap());
        assert!(!should_reclaim(FreelistStats {
            page_count: 0,
            freelist_count: 0,
            page_size,
        })
        .unwrap());
    }

    #[test]
    fn test_物理回収判定_整数演算のoverflowを発火扱いにしない() {
        assert!(matches!(
            should_reclaim(FreelistStats {
                page_count: u64::MAX,
                freelist_count: u64::MAX,
                page_size: u64::MAX,
            }),
            Err(MaintenanceFailure::ArithmeticOverflow)
        ));
    }

    #[test]
    fn test_起動時保守_非発火storeではvacuumと差し替えへ進まない() {
        let root = tempfile::TempDir::new().unwrap();
        let fault = Arc::new(FaultInjector::new());
        fault.arm_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto);
        let observer = Arc::new(RecordingObserver::default());
        let store = open_store_with_fault(root.path(), fault.clone(), observer.clone());
        let layout = StoreLayout::new(root.path());

        assert!(fault.take_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto));
        assert!(!observer.observed(AppDataPathOperation::Write, &layout.vacuum_database_path()));
        assert!(!layout.vacuum_database_path().exists());
        drop(store);
    }

    #[tokio::test]
    async fn test_起動時保守_発火storeを縮小してデータ保持と再writeを可能にする() {
        let root = tempfile::TempDir::new().unwrap();
        let (size_before, installation_id) = create_fragmented_store(root.path());
        let expected_snapshot = snapshot_store_path(root.path());
        let observer = Arc::new(RecordingObserver::default());

        let store = open_store_with_fault(
            root.path(),
            Arc::new(FaultInjector::new()),
            observer.clone(),
        );
        let size_after = std::fs::metadata(database_path(root.path())).unwrap().len();
        assert!(size_after < size_before);
        assert_eq!(store.installation_id(), installation_id);
        drop(store);
        assert_eq!(snapshot_store_path(root.path()), expected_snapshot);

        let store = open_store(root.path());
        let commit_id = CommitIdentity::parse("maintenance-new-write").unwrap();
        store
            .commit_batch(LocalAtomicBatch {
                commit_id: commit_id.clone(),
                idempotency: IdempotencyBinding {
                    installation_id: installation_id.clone(),
                    operation_kind: CommitOperationKind::Projection,
                    idempotency_key: "maintenance-new-write".to_string(),
                    payload_hash: [7; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: Vec::new(),
            })
            .await
            .unwrap();
        let layout = StoreLayout::new(root.path());
        for sidecar in layout.database_sidecar_paths() {
            assert!(observer.observed(AppDataPathOperation::Remove, &sidecar));
        }
        drop(store);

        assert_preserved_content(root.path(), &installation_id);
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        let new_commit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM logical_commits
                 WHERE commit_id = 'maintenance-new-write'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_commit_count, 1);
    }

    #[test]
    fn test_起動時保守_replace成功前の各faultで元storeを維持して一時artifactを除去する() {
        let seed = tempfile::TempDir::new().unwrap();
        let (seed_size, installation_id) = create_fragmented_store(seed.path());
        let seed_database = database_path(seed.path());
        for point in [
            MaintenanceFaultPoint::BeforeVacuumInto,
            MaintenanceFaultPoint::BeforeOutputValidation,
            MaintenanceFaultPoint::BeforeOutputPermission,
            MaintenanceFaultPoint::BeforeOutputSync,
            MaintenanceFaultPoint::BeforeCanonicalSidecarCleanup,
            MaintenanceFaultPoint::BeforeReplace,
        ] {
            let root = tempfile::TempDir::new().unwrap();
            let layout = StoreLayout::new(root.path());
            std::fs::copy(&seed_database, layout.database_path()).unwrap();
            let fault = Arc::new(FaultInjector::new());
            fault.arm_maintenance_fault(point);

            drop(open_store_with_fault(
                root.path(),
                fault,
                Arc::new(RecordingObserver::default()),
            ));

            assert_eq!(
                std::fs::metadata(layout.database_path()).unwrap().len(),
                seed_size
            );
            assert_preserved_content(root.path(), &installation_id);
            assert!(!layout.vacuum_database_path().exists());
            for sidecar in layout.vacuum_database_sidecar_paths() {
                assert!(!sidecar.exists(), "fault point {point:?}");
            }
        }
    }

    #[tokio::test]
    async fn test_起動時保守_replace直後のfault境界から新canonicalを次回openする() {
        let root = tempfile::TempDir::new().unwrap();
        let (size_before, installation_id) = create_fragmented_store(root.path());
        let expected_snapshot = snapshot_store_path(root.path());
        let layout = StoreLayout::new(root.path());
        let fault = Arc::new(FaultInjector::new());
        fault.arm_maintenance_fault(MaintenanceFaultPoint::AfterReplace);
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        prepare_vacuum_database(&layout, &connection, fault.as_ref()).unwrap();
        drop(connection);

        assert!(matches!(
            replace_canonical_database(&layout, fault.as_ref()),
            Err(CanonicalReplacementFailure::PostReplace(
                MaintenanceFailure::Injected(MaintenanceFaultPoint::AfterReplace)
            ))
        ));

        assert!(std::fs::metadata(database_path(root.path())).unwrap().len() < size_before);
        assert!(!layout.vacuum_database_path().exists());
        for sidecar in layout.database_sidecar_paths() {
            assert!(!sidecar.exists());
        }
        for sidecar in layout.vacuum_database_sidecar_paths() {
            assert!(!sidecar.exists());
        }

        let store = open_store(root.path());
        assert_eq!(store.installation_id(), installation_id);
        drop(store);
        assert_eq!(snapshot_store_path(root.path()), expected_snapshot);

        let store = open_store(root.path());
        let commit_id = CommitIdentity::parse("post-replace-new-write").unwrap();
        store
            .commit_batch(LocalAtomicBatch {
                commit_id,
                idempotency: IdempotencyBinding {
                    installation_id,
                    operation_kind: CommitOperationKind::Projection,
                    idempotency_key: "post-replace-new-write".to_string(),
                    payload_hash: [9; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: Vec::new(),
            })
            .await
            .unwrap();
    }

    #[test]
    fn test_起動時保守_vacuum書込み前からowner_only権限を持つ() {
        let root = tempfile::TempDir::new().unwrap();
        create_fragmented_store(root.path());
        let layout = StoreLayout::new(root.path());
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        let fault = FaultInjector::new();
        fault.arm_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto);

        assert!(matches!(
            prepare_vacuum_database(&layout, &connection, &fault),
            Err(MaintenanceFailure::Injected(
                MaintenanceFaultPoint::BeforeVacuumInto
            ))
        ));
        let vacuum_path = layout.vacuum_database_path();
        assert_eq!(std::fs::metadata(&vacuum_path).unwrap().len(), 0);
        verify_owner_only_permissions(&vacuum_path).unwrap();
        cleanup_vacuum_artifacts(&layout).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_起動時保守_非utf8一時path失敗を区別して元storeを継続する() {
        use std::os::unix::ffi::OsStringExt;

        let root = tempfile::TempDir::new().unwrap();
        let (_, installation_id) = create_fragmented_store(root.path());
        let connection = open_existing_writer(&database_path(root.path())).unwrap();
        let invalid_root =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()));
        let layout = StoreLayout::new(&invalid_root);

        let error = prepare_vacuum_database(&layout, &connection, &FaultInjector::new())
            .expect_err("non-UTF-8 vacuum path must be rejected");
        let stored_installation_id: String = connection
            .query_row(
                "SELECT installation_id FROM store_metadata WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_installation_id, installation_id);
        assert_eq!(error.to_string(), "vacuum database path is not valid UTF-8");
        assert!(matches!(error, MaintenanceFailure::InvalidPathEncoding));
        assert_ne!(
            error.to_string(),
            MaintenanceFailure::InvalidPragmaValue.to_string()
        );
        assert!(!layout.vacuum_database_path().exists());
    }

    fn write_stale_vacuum_artifacts(layout: &StoreLayout) {
        std::fs::write(layout.vacuum_database_path(), b"stale-database").unwrap();
        for (index, sidecar) in layout
            .vacuum_database_sidecar_paths()
            .into_iter()
            .enumerate()
        {
            std::fs::write(sidecar, format!("stale-sidecar-{index}")).unwrap();
        }
    }

    #[test]
    fn test_起動時保守_非発火時は前回のstale一時artifactだけを除去する() {
        let root = tempfile::TempDir::new().unwrap();
        let installation_id = open_store(root.path()).installation_id().to_string();
        let layout = StoreLayout::new(root.path());
        write_stale_vacuum_artifacts(&layout);
        let fault = Arc::new(FaultInjector::new());
        fault.arm_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto);
        let observer = Arc::new(RecordingObserver::default());

        let store = open_store_with_fault(root.path(), fault.clone(), observer.clone());

        assert_eq!(store.installation_id(), installation_id);
        assert!(fault.take_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto));
        assert!(!observer.observed(AppDataPathOperation::Write, &layout.vacuum_database_path()));
        assert!(!layout.vacuum_database_path().exists());
        for sidecar in layout.vacuum_database_sidecar_paths() {
            assert!(!sidecar.exists());
        }
    }

    #[test]
    fn test_起動時保守_発火時はstale一時artifactを除去して回収を完了する() {
        let root = tempfile::TempDir::new().unwrap();
        let (size_before, installation_id) = create_fragmented_store(root.path());
        let expected_snapshot = snapshot_store_path(root.path());
        let layout = StoreLayout::new(root.path());
        write_stale_vacuum_artifacts(&layout);

        let store = open_store(root.path());

        assert_eq!(store.installation_id(), installation_id);
        assert!(std::fs::metadata(database_path(root.path())).unwrap().len() < size_before);
        assert!(!layout.vacuum_database_path().exists());
        for sidecar in layout.vacuum_database_sidecar_paths() {
            assert!(!sidecar.exists());
        }
        drop(store);
        assert_eq!(snapshot_store_path(root.path()), expected_snapshot);
    }

    #[test]
    fn test_起動時保守_checkpoint_busyではwalを保持して保守をskipする() {
        let root = tempfile::TempDir::new().unwrap();
        let store = open_store(root.path());
        let installation_id = store.installation_id().to_string();
        drop(store);
        let layout = StoreLayout::new(root.path());
        let database_path = layout.database_path();
        let reader = open_reader(&database_path).unwrap();
        reader.execute_batch("BEGIN;").unwrap();
        reader
            .query_row("SELECT COUNT(*) FROM store_metadata", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let writer = open_existing_writer(&database_path).unwrap();
        writer
            .execute(
                "INSERT INTO logical_commits (
                     commit_id, installation_id, operation_kind, idempotency_key,
                     payload_hash, state, first_global_sequence, last_global_sequence,
                     event_count, mutation_count, stream_heads_json, result_hash,
                     committed_at_ms
                 ) VALUES (
                     'checkpoint-busy', ?1, 'projection', 'checkpoint-busy',
                     zeroblob(32), 'sealed', NULL, NULL, 0, 0, '{}', NULL, 2
                 )",
                [&installation_id],
            )
            .unwrap();
        drop(writer);
        let fault = Arc::new(FaultInjector::new());
        fault.arm_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto);
        let observer = Arc::new(RecordingObserver::default());

        let store = open_store_with_fault(root.path(), fault.clone(), observer.clone());

        assert!(fault.take_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto));
        assert!(!observer.observed(AppDataPathOperation::Write, &layout.vacuum_database_path()));
        for sidecar in layout.database_sidecar_paths() {
            assert!(sidecar.exists());
            assert!(!observer.observed(AppDataPathOperation::Remove, &sidecar));
        }
        let main_only_path = root.path().join("main-only.sqlite3");
        std::fs::copy(&database_path, &main_only_path).unwrap();
        let mut main_only_uri = url::Url::from_file_path(&main_only_path).unwrap();
        main_only_uri
            .query_pairs_mut()
            .append_pair("immutable", "1");
        let main_only = Connection::open_with_flags(
            main_only_uri.as_str(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
                | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        let main_only_count: i64 = main_only
            .query_row(
                "SELECT COUNT(*) FROM logical_commits WHERE commit_id = 'checkpoint-busy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(main_only_count, 0);
        let verification = open_reader(&database_path).unwrap();
        let retained: i64 = verification
            .query_row(
                "SELECT COUNT(*) FROM logical_commits WHERE commit_id = 'checkpoint-busy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, 1);
        drop(store);
        reader.execute_batch("ROLLBACK;").unwrap();
    }
}
