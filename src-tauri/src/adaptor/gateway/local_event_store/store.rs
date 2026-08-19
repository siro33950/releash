//! `LocalEventStore`: the SQLite implementation of
//! `LocalEventTransactionRepository`, the single mutation authority.
//!
//! One dedicated writer thread and up to four dedicated reader threads own
//! every rusqlite call. The async trait methods only validate, encode, and
//! exchange messages with those threads.

use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

use crate::adaptor::gateway::local_event_store::clock::{StoreClock, SystemStoreClock};
use crate::adaptor::gateway::local_event_store::commit::{execute_commit, resolve_commit_row};
use crate::adaptor::gateway::local_event_store::connection::{
    check_sqlite_version, open_existing_writer, open_reader, open_writer,
    set_owner_only_permissions, ConnectionError,
};
use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::fault::InitialCreateFaultPoint;
use crate::adaptor::gateway::local_event_store::layout::{
    create_initial_create_evidence_with_fault, inspect_initial_create_evidence,
    remove_initial_create_evidence, replace_invalid_evidence_for_absent_database_with_fault,
    sqlite_sidecar_paths, InitialCreateEvidenceState, NoopStorePathObserver, StoreLayout,
    StorePathObserver, StorePathOperation,
};
use crate::adaptor::gateway::local_event_store::maintenance::{
    run_startup_maintenance, StartupMaintenanceError,
};
use crate::adaptor::gateway::local_event_store::node_events::{self, NewNodeEventRow};
use crate::adaptor::gateway::local_event_store::projection_record_codec::canonical_mutation_identity_v1 as canonical_projection_mutation_identity_v1;
use crate::adaptor::gateway::local_event_store::reader::{
    load_stream_page, run_query, QueryContext, ReaderPool, RecoverySnapshotPager, READER_POOL_SIZE,
};
use crate::adaptor::gateway::local_event_store::schema::{
    evolve_schema, initialize_schema, validate_current_schema, validate_supported_schema_v1,
    InitialStoreMetadata, APPLICATION_ID, CURRENT_SCHEMA_VERSION,
};
use crate::adaptor::gateway::local_event_store::writer::{
    AdmitRejection, CommitWriteRequest, NodeEventAppendRequest, NodeEventTreeDeleteRequest,
    NodeEventWriteError, PreparedBatch, PreparedEvent, PreparedNodeEvent, QueuePop, WriteQueue,
    WriteRequest, MAX_BATCH_DECODED_BYTES, MAX_BATCH_EVENTS, MAX_BATCH_STATE_MUTATIONS,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    LoadStreamRequest, LocalAtomicBatch, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    SafeOperationFailure, SessionOperationFailureKind,
};

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn random_key_32() -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key
}

fn sqlite_error_is_storage_unavailable(error: &rusqlite::Error) -> bool {
    let rusqlite::Error::SqliteFailure(inner, _) = error else {
        return false;
    };
    matches!(
        inner.code,
        rusqlite::ErrorCode::PermissionDenied
            | rusqlite::ErrorCode::DatabaseBusy
            | rusqlite::ErrorCode::DatabaseLocked
            | rusqlite::ErrorCode::OutOfMemory
            | rusqlite::ErrorCode::ReadOnly
            | rusqlite::ErrorCode::OperationInterrupted
            | rusqlite::ErrorCode::SystemIoFailure
            | rusqlite::ErrorCode::DiskFull
            | rusqlite::ErrorCode::CannotOpen
            | rusqlite::ErrorCode::FileLockingProtocolFailed
            | rusqlite::ErrorCode::TooBig
            | rusqlite::ErrorCode::NoLargeFileSupport
            | rusqlite::ErrorCode::AuthorizationForStatementDenied
    )
}

fn classify_sqlite_error(
    error: &rusqlite::Error,
    otherwise: LocalEventStoreOpenError,
) -> LocalEventStoreOpenError {
    if sqlite_error_is_storage_unavailable(error) {
        LocalEventStoreOpenError::StorageUnavailable
    } else {
        otherwise
    }
}

fn classify_connection_error(
    error: &ConnectionError,
    otherwise: LocalEventStoreOpenError,
) -> LocalEventStoreOpenError {
    match error {
        ConnectionError::SqliteTooOld { .. } => LocalEventStoreOpenError::UnsupportedRuntime,
        ConnectionError::Sqlite(error) => classify_sqlite_error(error, otherwise),
    }
}

fn classify_startup_maintenance_error(error: &StartupMaintenanceError) -> LocalEventStoreOpenError {
    match error {
        StartupMaintenanceError::Connection(error) => {
            classify_connection_error(error, LocalEventStoreOpenError::StoreValidationFailed)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WalCheckpointResult {
    busy: bool,
    log_frames: i64,
    checkpointed_frames: i64,
}

fn truncate_wal_checkpoint(
    connection: &rusqlite::Connection,
) -> Result<WalCheckpointResult, rusqlite::Error> {
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok(WalCheckpointResult {
            busy: row.get::<_, i64>(0)? != 0,
            log_frames: row.get(1)?,
            checkpointed_frames: row.get(2)?,
        })
    })
}

fn classify_writer_lock_error(error: &std::io::Error) -> LocalEventStoreOpenError {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        LocalEventStoreOpenError::WriterLockHeld
    } else {
        LocalEventStoreOpenError::StorageUnavailable
    }
}

fn sqlite_header_is_valid(
    layout: &StoreLayout,
    path: &std::path::Path,
) -> Result<bool, LocalEventStoreOpenError> {
    use std::io::Read;

    layout.observe(StorePathOperation::Open, path);
    layout.observe(StorePathOperation::Read, path);
    let mut file =
        std::fs::File::open(path).map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header == b"SQLite format 3\0"),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(_) => Err(LocalEventStoreOpenError::StorageUnavailable),
    }
}

fn table_columns(
    connection: &rusqlite::Connection,
    table: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}

fn open_schema_inspection(
    layout: &StoreLayout,
    path: &std::path::Path,
) -> Result<rusqlite::Connection, LocalEventStoreOpenError> {
    // Classification reads the fixed authority directly. SQLite's
    // `readonly_shm` URI mode sees committed WAL frames while mapping the
    // fixed SHM wal-index read-only, so a closed classification failure does
    // not claim a read-mark or change a sidecar byte. `immutable=1` is never
    // used when a non-empty WAL exists because it could ignore committed
    // frames. No create flag is permitted at this boundary.
    let [wal_path, shm_path] = sqlite_sidecar_paths(path);
    let mut wal_has_bytes = false;
    for sidecar in [wal_path.clone(), shm_path] {
        layout.observe(StorePathOperation::Metadata, &sidecar);
        match std::fs::metadata(&sidecar) {
            Ok(metadata) => {
                layout.observe(StorePathOperation::Open, &sidecar);
                layout.observe(StorePathOperation::Read, &sidecar);
                if sidecar == wal_path {
                    wal_has_bytes = metadata.len() > 0;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(LocalEventStoreOpenError::StorageUnavailable),
        }
    }
    layout.observe(StorePathOperation::Open, path);
    layout.observe(StorePathOperation::Read, path);
    let mut uri = url::Url::from_file_path(path)
        .map_err(|()| LocalEventStoreOpenError::StorageUnavailable)?;
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
    )
    .map_err(|error| {
        classify_sqlite_error(&error, LocalEventStoreOpenError::InitializationStateInvalid)
    })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::InitializationStateInvalid)
        })?;
    Ok(connection)
}

fn is_proven_initial_create_residue(
    layout: &StoreLayout,
    path: &std::path::Path,
) -> Result<bool, LocalEventStoreOpenError> {
    layout.observe(StorePathOperation::Metadata, path);
    let length = std::fs::metadata(path)
        .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?
        .len();
    if length == 0 {
        return Ok(true);
    }
    if !sqlite_header_is_valid(layout, path)? {
        return Ok(false);
    }
    layout.observe(StorePathOperation::Open, path);
    layout.observe(StorePathOperation::Read, path);
    let connection = open_schema_inspection(layout, path)?;
    let application_table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::InitializationStateInvalid)
        })?;
    Ok(application_table_count == 0)
}

fn remove_initial_create_database(layout: &StoreLayout) -> Result<(), LocalEventStoreOpenError> {
    let database = layout.database_path();
    let [wal_path, shm_path] = layout.database_sidecar_paths();
    for path in [database, wal_path, shm_path] {
        layout.observe(StorePathOperation::Remove, &path);
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(LocalEventStoreOpenError::StorageUnavailable),
        }
    }
    layout
        .sync_app_data_root()
        .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingDatabaseKind {
    Current,
    SupportedV1,
    SupportedV2,
    SupportedV3,
    SupportedV4,
    SupportedV5,
}

fn classify_existing_database(
    layout: &StoreLayout,
    path: &std::path::Path,
) -> Result<ExistingDatabaseKind, LocalEventStoreOpenError> {
    layout.observe(StorePathOperation::Metadata, path);
    let length = std::fs::metadata(path)
        .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?
        .len();
    if length == 0 || !sqlite_header_is_valid(layout, path)? {
        return Err(LocalEventStoreOpenError::InitializationStateInvalid);
    }
    layout.observe(StorePathOperation::Open, path);
    layout.observe(StorePathOperation::Read, path);
    let connection = open_schema_inspection(layout, path)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::InitializationStateInvalid)
        })?;
    let application_id = connection
        .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
        .map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::InitializationStateInvalid)
        })?;
    let user_version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| {
            classify_sqlite_error(
                &error,
                if application_id == i64::from(APPLICATION_ID) {
                    LocalEventStoreOpenError::StoreValidationFailed
                } else {
                    LocalEventStoreOpenError::InitializationStateInvalid
                },
            )
        })?;
    let columns = table_columns(&connection, "store_metadata").map_err(|error| {
        classify_sqlite_error(
            &error,
            if application_id == i64::from(APPLICATION_ID) {
                LocalEventStoreOpenError::StoreValidationFailed
            } else {
                LocalEventStoreOpenError::InitializationStateInvalid
            },
        )
    })?;
    let supported_v1_signature = columns.iter().any(|column| column == "store_id")
        && columns.iter().any(|column| column == "generation_id")
        && columns.iter().any(|column| column == "boot_id");
    if supported_v1_signature {
        if application_id != 0 && application_id != i64::from(APPLICATION_ID) {
            return Err(LocalEventStoreOpenError::StoreValidationFailed);
        }
        if user_version != 0 && user_version != 1 {
            return Err(LocalEventStoreOpenError::UnsupportedStoreVersion);
        }
        validate_supported_schema_v1(&connection).map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
        })?;
        return Ok(ExistingDatabaseKind::SupportedV1);
    }
    if application_id == i64::from(APPLICATION_ID) {
        if matches!(user_version, 2..=5)
            && columns.iter().any(|column| column == "installation_id")
            && !columns.iter().any(|column| column == "store_id")
        {
            return Ok(match user_version {
                2 => ExistingDatabaseKind::SupportedV2,
                3 => ExistingDatabaseKind::SupportedV3,
                4 => ExistingDatabaseKind::SupportedV4,
                5 => ExistingDatabaseKind::SupportedV5,
                _ => unreachable!("supported schema version was matched above"),
            });
        }
        if user_version != CURRENT_SCHEMA_VERSION {
            return Err(LocalEventStoreOpenError::UnsupportedStoreVersion);
        }
        validate_current_schema(&connection).map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
        })?;
        return Ok(ExistingDatabaseKind::Current);
    }
    if columns.iter().any(|column| column == "installation_id")
        || columns.iter().any(|column| column == "store_id")
    {
        return Err(LocalEventStoreOpenError::StoreValidationFailed);
    }
    Err(LocalEventStoreOpenError::InitializationStateInvalid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalEventStoreOpenError {
    WriterLockHeld,
    StorageUnavailable,
    UnsupportedRuntime,
    UnsupportedStoreVersion,
    InitializationStateInvalid,
    StoreValidationFailed,
    SchemaEvolutionFailed,
}

impl std::fmt::Display for LocalEventStoreOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriterLockHeld => write!(f, "another process holds the writer lock"),
            Self::StorageUnavailable => write!(f, "local event storage is unavailable"),
            Self::UnsupportedRuntime => write!(f, "the bundled SQLite runtime is unsupported"),
            Self::UnsupportedStoreVersion => {
                write!(f, "the local event store version is unsupported")
            }
            Self::InitializationStateInvalid => {
                write!(f, "the local event store initialization state is invalid")
            }
            Self::StoreValidationFailed => {
                write!(f, "the local event store could not be validated")
            }
            Self::SchemaEvolutionFailed => {
                write!(f, "the local event store schema could not be evolved")
            }
        }
    }
}

impl std::error::Error for LocalEventStoreOpenError {}

pub struct LocalEventStoreConfig {
    pub app_data_root: PathBuf,
    pub clock: Arc<dyn StoreClock>,
    pub registry: Arc<EventCodecRegistry>,
    pub fault: Arc<FaultInjector>,
    pub path_observer: Arc<dyn StorePathObserver>,
}

impl LocalEventStoreConfig {
    /// Production configuration: system clock, default registry, no faults.
    pub fn production(app_data_root: PathBuf) -> Self {
        Self {
            app_data_root,
            clock: Arc::new(SystemStoreClock),
            registry: Arc::new(EventCodecRegistry::new()),
            fault: Arc::new(FaultInjector::new()),
            path_observer: Arc::new(NoopStorePathObserver),
        }
    }
}

/// The permanent SQLite local event store.
pub struct LocalEventStore {
    registry: Arc<EventCodecRegistry>,
    #[cfg(test)]
    fault: Arc<FaultInjector>,
    queue: Arc<WriteQueue>,
    readers: Arc<ReaderPool>,
    recovery_snapshots: Arc<RecoverySnapshotPager>,
    query_context: Arc<QueryContext>,
    installation_id: String,
    operation_binding_key: [u8; 32],
    writer_worker: Option<std::thread::JoinHandle<()>>,
    reader_workers: Vec<std::thread::JoinHandle<()>>,
    // Held for the lifetime of the store: exclusive app-data writer lock.
    _writer_lock: std::fs::File,
}

impl LocalEventStore {
    /// Open (or bootstrap) the store under `app_data_root`.
    ///
    /// Create or open the single fixed-path SQLite authority.
    pub fn open(config: LocalEventStoreConfig) -> Result<Arc<Self>, LocalEventStoreOpenError> {
        check_sqlite_version().map_err(|error| {
            classify_connection_error(&error, LocalEventStoreOpenError::StorageUnavailable)
        })?;
        let layout =
            StoreLayout::with_observer(&config.app_data_root, Arc::clone(&config.path_observer));
        layout
            .ensure_app_data_root()
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;

        let lock_path = layout.writer_lock_path();
        layout.observe(StorePathOperation::Open, &lock_path);
        layout.observe(StorePathOperation::Write, &lock_path);
        let writer_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        layout.observe(StorePathOperation::Metadata, &lock_path);
        set_owner_only_permissions(&lock_path)
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        fs2::FileExt::try_lock_exclusive(&writer_lock)
            .map_err(|error| classify_writer_lock_error(&error))?;

        let database_path = layout.database_path();
        let evidence = inspect_initial_create_evidence(&layout)
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        layout.observe(StorePathOperation::Metadata, &database_path);
        let database_exists = database_path
            .try_exists()
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        let process_instance_id = uuid::Uuid::new_v4().to_string();
        let now_ms = config.clock.now_ms().max(0);

        let writer_connection = if !database_exists {
            match evidence {
                InitialCreateEvidenceState::Absent => {
                    create_initial_create_evidence_with_fault(&layout, Some(config.fault.as_ref()))
                }
                InitialCreateEvidenceState::Invalid => {
                    replace_invalid_evidence_for_absent_database_with_fault(
                        &layout,
                        Some(config.fault.as_ref()),
                    )
                }
                InitialCreateEvidenceState::Valid => Ok(()),
            }
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
            layout.observe(StorePathOperation::Open, &database_path);
            layout.observe(StorePathOperation::Write, &database_path);
            let connection = open_writer(&database_path).map_err(|error| {
                classify_connection_error(&error, LocalEventStoreOpenError::SchemaEvolutionFailed)
            })?;
            if config
                .fault
                .take_initial_create_fault(InitialCreateFaultPoint::AfterSqliteFileCreate)
            {
                #[cfg(test)]
                config.fault.crash_initial_create_process_if_armed(
                    InitialCreateFaultPoint::AfterSqliteFileCreate,
                );
                return Err(LocalEventStoreOpenError::StorageUnavailable);
            }
            layout.observe(StorePathOperation::Metadata, &database_path);
            set_owner_only_permissions(&database_path)
                .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
            let installation_id = config
                .fault
                .initial_installation_id()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let cursor_key = random_key_32();
            let operation_binding_key = random_key_32();
            initialize_schema(
                &connection,
                &InitialStoreMetadata {
                    installation_id: &installation_id,
                    cursor_hmac_key: &cursor_key,
                    operation_binding_hmac_key: &operation_binding_key,
                    process_instance_id: &process_instance_id,
                    created_at_ms: now_ms,
                },
                config.fault.as_ref(),
            )
            .map_err(|error| {
                classify_sqlite_error(&error, LocalEventStoreOpenError::SchemaEvolutionFailed)
            })?;
            connection
        } else {
            if evidence == InitialCreateEvidenceState::Valid
                && is_proven_initial_create_residue(&layout, &database_path)?
            {
                remove_initial_create_database(&layout)?;
                layout.observe(StorePathOperation::Open, &database_path);
                layout.observe(StorePathOperation::Write, &database_path);
                let connection = open_writer(&database_path).map_err(|error| {
                    classify_connection_error(
                        &error,
                        LocalEventStoreOpenError::SchemaEvolutionFailed,
                    )
                })?;
                if config
                    .fault
                    .take_initial_create_fault(InitialCreateFaultPoint::AfterSqliteFileCreate)
                {
                    #[cfg(test)]
                    config.fault.crash_initial_create_process_if_armed(
                        InitialCreateFaultPoint::AfterSqliteFileCreate,
                    );
                    return Err(LocalEventStoreOpenError::StorageUnavailable);
                }
                layout.observe(StorePathOperation::Metadata, &database_path);
                set_owner_only_permissions(&database_path)
                    .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
                let installation_id = config
                    .fault
                    .initial_installation_id()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let cursor_key = random_key_32();
                let operation_binding_key = random_key_32();
                initialize_schema(
                    &connection,
                    &InitialStoreMetadata {
                        installation_id: &installation_id,
                        cursor_hmac_key: &cursor_key,
                        operation_binding_hmac_key: &operation_binding_key,
                        process_instance_id: &process_instance_id,
                        created_at_ms: now_ms,
                    },
                    config.fault.as_ref(),
                )
                .map_err(|error| {
                    classify_sqlite_error(&error, LocalEventStoreOpenError::SchemaEvolutionFailed)
                })?;
                connection
            } else {
                let kind = classify_existing_database(&layout, &database_path)?;
                layout.observe(StorePathOperation::Open, &database_path);
                layout.observe(StorePathOperation::Write, &database_path);
                let connection = open_existing_writer(&database_path).map_err(|error| {
                    classify_connection_error(
                        &error,
                        LocalEventStoreOpenError::StoreValidationFailed,
                    )
                })?;
                if matches!(
                    kind,
                    ExistingDatabaseKind::SupportedV1
                        | ExistingDatabaseKind::SupportedV2
                        | ExistingDatabaseKind::SupportedV3
                        | ExistingDatabaseKind::SupportedV4
                        | ExistingDatabaseKind::SupportedV5
                ) {
                    evolve_schema(&connection, config.fault.as_ref()).map_err(|error| {
                        classify_sqlite_error(
                            &error,
                            LocalEventStoreOpenError::SchemaEvolutionFailed,
                        )
                    })?;
                }
                connection
            }
        };

        validate_current_schema(&writer_connection).map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
        })?;
        writer_connection
            .execute(
                "UPDATE store_metadata SET process_instance_id = ?1 WHERE id = 1",
                rusqlite::params![process_instance_id],
            )
            .map_err(|error| {
                classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
            })?;
        let checkpoint = truncate_wal_checkpoint(&writer_connection).map_err(|error| {
            classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
        })?;
        if checkpoint.busy {
            log::warn!(
                "local event store startup maintenance skipped because WAL checkpoint is busy: log_frames={}, checkpointed_frames={}",
                checkpoint.log_frames,
                checkpoint.checkpointed_frames
            );
        }
        layout.observe(StorePathOperation::Open, &database_path);
        layout.observe(StorePathOperation::Sync, &database_path);
        std::fs::File::open(&database_path)
            .and_then(|file| file.sync_all())
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        if config
            .fault
            .take_initial_create_fault(InitialCreateFaultPoint::AfterDatabaseSync)
        {
            #[cfg(test)]
            config
                .fault
                .crash_initial_create_process_if_armed(InitialCreateFaultPoint::AfterDatabaseSync);
            return Err(LocalEventStoreOpenError::StorageUnavailable);
        }
        if config
            .fault
            .take_initial_create_fault(InitialCreateFaultPoint::BeforeEvidenceUnlink)
        {
            #[cfg(test)]
            config.fault.crash_initial_create_process_if_armed(
                InitialCreateFaultPoint::BeforeEvidenceUnlink,
            );
            return Err(LocalEventStoreOpenError::StorageUnavailable);
        }
        remove_initial_create_evidence(&layout)
            .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
        if config
            .fault
            .take_initial_create_fault(InitialCreateFaultPoint::AfterEvidenceUnlink)
        {
            #[cfg(test)]
            config.fault.crash_initial_create_process_if_armed(
                InitialCreateFaultPoint::AfterEvidenceUnlink,
            );
            return Err(LocalEventStoreOpenError::StorageUnavailable);
        }

        let writer_connection = if checkpoint.busy {
            writer_connection
        } else {
            run_startup_maintenance(&layout, writer_connection, config.fault.as_ref()).map_err(
                |error| {
                    let correlation = correlation_id();
                    log::error!(
                        "local event store could not reopen after startup maintenance [{correlation}]: {error}"
                    );
                    classify_startup_maintenance_error(&error)
                },
            )?
        };

        let (installation_id, cursor_key, operation_binding_key): (String, Vec<u8>, Vec<u8>) =
            writer_connection
                .query_row(
                    "SELECT installation_id, cursor_hmac_key, operation_binding_hmac_key
                     FROM store_metadata WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(|error| {
                    classify_sqlite_error(&error, LocalEventStoreOpenError::StoreValidationFailed)
                })?;
        let operation_binding_key: [u8; 32] = operation_binding_key
            .try_into()
            .map_err(|_| LocalEventStoreOpenError::StoreValidationFailed)?;

        let queue = WriteQueue::new();
        let readers = ReaderPool::new(Arc::clone(&config.clock));
        let query_context = Arc::new(QueryContext {
            registry: Arc::clone(&config.registry),
            cursor_key,
            process_instance_id: process_instance_id.clone(),
            clock: Arc::clone(&config.clock),
        });
        let recovery_snapshots =
            RecoverySnapshotPager::new(database_path.clone(), Arc::clone(&query_context));

        let mut reader_connections = Vec::with_capacity(READER_POOL_SIZE);
        for index in 0..READER_POOL_SIZE {
            layout.observe(StorePathOperation::Open, &database_path);
            layout.observe(StorePathOperation::Read, &database_path);
            let connection = open_reader(&database_path)
                .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?;
            reader_connections.push((index, connection));
        }

        let writer_worker = {
            let queue = Arc::clone(&queue);
            let fault = Arc::clone(&config.fault);
            let clock = Arc::clone(&config.clock);
            std::thread::Builder::new()
                .name("local-event-store-writer".to_string())
                .spawn(move || loop {
                    match queue.pop_with_timeout(std::time::Duration::from_secs(1)) {
                        QueuePop::Request(request) => match *request {
                            WriteRequest::Commit(request) => {
                                let result = execute_commit(
                                    &writer_connection,
                                    &request.prepared,
                                    clock.now_ms().max(0),
                                    &fault,
                                );
                                if fault.take_drop_reply() {
                                    drop(request.reply);
                                } else {
                                    let _ = request.reply.send(result);
                                }
                            }
                            WriteRequest::NodeEventAppend(request) => {
                                let timestamp_ms = request
                                    .timestamp_ms
                                    .unwrap_or_else(|| clock.now_ms())
                                    .max(0);
                                let result = node_events::append_node_event(
                                    &writer_connection,
                                    &request.row,
                                    timestamp_ms,
                                )
                                .map_err(|error| {
                                    let correlation = correlation_id();
                                    log::error!(
                                        "node event append failed [{correlation}]: {error}"
                                    );
                                    NodeEventWriteError::StorageUnavailable
                                });
                                let _ = request.reply.send(result);
                            }
                            WriteRequest::NodeEventTreeDelete(request) => {
                                let result =
                                    node_events::delete_tree(&writer_connection, &request.tree_id)
                                        .map_err(|error| {
                                            let correlation = correlation_id();
                                            log::error!(
                                        "node event tree delete failed [{correlation}]: {error}"
                                    );
                                            NodeEventWriteError::StorageUnavailable
                                        });
                                let _ = request.reply.send(result);
                            }
                        },
                        QueuePop::Idle => {}
                        QueuePop::Closed => break,
                    }
                })
                .map_err(|_| LocalEventStoreOpenError::StorageUnavailable)?
        };

        let mut writer_worker = Some(writer_worker);
        let mut reader_workers: Vec<std::thread::JoinHandle<()>> =
            Vec::with_capacity(READER_POOL_SIZE);
        for (index, connection) in reader_connections {
            let worker_readers = Arc::clone(&readers);
            let worker = match std::thread::Builder::new()
                .name(format!("local-event-store-reader-{index}"))
                .spawn(move || worker_readers.run_worker(connection))
            {
                Ok(worker) => worker,
                Err(_) => {
                    queue.close();
                    readers.close();
                    for worker in reader_workers {
                        let _ = worker.join();
                    }
                    if let Some(worker) = writer_worker.take() {
                        let _ = worker.join();
                    }
                    return Err(LocalEventStoreOpenError::StorageUnavailable);
                }
            };
            reader_workers.push(worker);
        }

        Ok(Arc::new(Self {
            registry: Arc::clone(&config.registry),
            #[cfg(test)]
            fault: config.fault,
            queue,
            readers,
            recovery_snapshots,
            query_context,
            installation_id,
            operation_binding_key,
            writer_worker,
            reader_workers,
            _writer_lock: writer_lock,
        }))
    }

    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn process_instance_id(&self) -> &str {
        &self.query_context.process_instance_id
    }

    #[cfg(test)]
    pub fn fault_injector(&self) -> &Arc<FaultInjector> {
        &self.fault
    }

    /// Validate and encode a batch before queue admission (design step 1).
    fn prepare(&self, batch: LocalAtomicBatch) -> Result<PreparedBatch, CommitBatchError> {
        if batch.idempotency.installation_id != self.installation_id {
            return Err(
                self.shape_error("batch installation identity does not match the store authority")
            );
        }
        if batch.events.len() > MAX_BATCH_EVENTS
            || batch.state_mutations.len() > MAX_BATCH_STATE_MUTATIONS
        {
            return Err(CommitBatchError::CapacityExceeded);
        }
        // Every stream a batch changes appears exactly once in expected_heads,
        // and every event stream is declared.
        for (index, head) in batch.expected_heads.iter().enumerate() {
            if batch.expected_heads[..index]
                .iter()
                .any(|other| other.stream_id == head.stream_id)
            {
                return Err(self.shape_error("duplicate expected stream head"));
            }
        }
        for event in &batch.events {
            if !batch
                .expected_heads
                .iter()
                .any(|head| head.stream_id == event.stream_id)
            {
                return Err(self.shape_error("event stream missing from expected heads"));
            }
        }

        let mut decoded_bytes = 0usize;
        let mut prepared_events = Vec::with_capacity(batch.events.len());
        for event in &batch.events {
            let payload = self.registry.encode(&event.event).map_err(|error| {
                let correlation = correlation_id();
                log::error!("local event store payload encode failed [{correlation}]: {error}");
                CommitBatchError::Corrupt {
                    correlation_id: correlation,
                }
            })?;
            decoded_bytes += payload.payload.len() + 128;
            let payload_sha256: [u8; 32] = Sha256::digest(&payload.payload).into();
            prepared_events.push(PreparedEvent {
                stream_id: event.stream_id.clone(),
                payload,
                payload_sha256,
                occurred_at_ms: event.occurred_at_ms,
            });
        }
        for mutation in &batch.state_mutations {
            decoded_bytes += mutation.approximate_bytes();
        }
        if decoded_bytes > MAX_BATCH_DECODED_BYTES {
            return Err(CommitBatchError::CapacityExceeded);
        }

        let critical = batch.idempotency.operation_kind.is_critical()
            || batch
                .state_mutations
                .iter()
                .any(LocalStateMutation::is_critical);
        Ok(PreparedBatch {
            batch,
            events: prepared_events,
            node_events: Vec::new(),
            decoded_bytes,
            critical,
        })
    }

    pub(crate) async fn commit_batch_with_node_events(
        &self,
        batch: LocalAtomicBatch,
        node_events: Vec<PreparedNodeEvent>,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        if node_events.len() > MAX_BATCH_EVENTS {
            return Err(CommitBatchError::CapacityExceeded);
        }
        let identity = batch.commit_id.clone();
        let mut prepared = self.prepare(batch)?;
        for event in &node_events {
            prepared.decoded_bytes = prepared
                .decoded_bytes
                .saturating_add(event.row.detail.len().saturating_add(256));
        }
        if prepared.decoded_bytes > MAX_BATCH_DECODED_BYTES {
            return Err(CommitBatchError::CapacityExceeded);
        }
        prepared.node_events = node_events;
        let (reply, receiver) = oneshot::channel();
        match self
            .queue
            .admit(WriteRequest::Commit(CommitWriteRequest { prepared, reply }))
        {
            Ok(()) => {}
            Err(AdmitRejection::Capacity) => return Err(CommitBatchError::CapacityExceeded),
            Err(AdmitRejection::Closed) => {
                return Err(CommitBatchError::OutcomeUnknown { identity });
            }
        }
        match receiver.await {
            Ok(result) => result,
            Err(_) => Err(CommitBatchError::OutcomeUnknown { identity }),
        }
    }

    fn shape_error(&self, context: &str) -> CommitBatchError {
        let correlation = correlation_id();
        log::error!("local event store batch shape invalid [{correlation}]: {context}");
        CommitBatchError::Corrupt {
            correlation_id: correlation,
        }
    }

    async fn submit_query<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let receiver = self.readers.submit(run)?;
        receiver
            .await
            .map_err(|_| LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "local event store reader reply lost",
                    correlation_id(),
                ),
            })?
    }

    /// Append one fact row to the unified-node fact log. The write is a
    /// single-row INSERT serialized on the writer thread; atomicity never
    /// spans more than this one row.
    ///
    /// `timestamp_ms` は事実の発生時刻。None なら store の clock で刻む。
    pub(crate) async fn append_node_event(
        &self,
        row: NewNodeEventRow,
        timestamp_ms: Option<i64>,
    ) -> Result<i64, NodeEventWriteError> {
        let (reply, receiver) = oneshot::channel();
        match self
            .queue
            .admit(WriteRequest::NodeEventAppend(NodeEventAppendRequest {
                row,
                timestamp_ms,
                reply,
            })) {
            Ok(()) => {}
            Err(AdmitRejection::Capacity) => return Err(NodeEventWriteError::StorageUnavailable),
            Err(AdmitRejection::Closed) => return Err(NodeEventWriteError::OutcomeUnknown),
        }
        receiver
            .await
            .map_err(|_| NodeEventWriteError::OutcomeUnknown)?
    }

    /// Physically delete one tree from the unified-node fact log.
    pub(crate) async fn delete_node_event_tree(
        &self,
        tree_id: String,
    ) -> Result<u64, NodeEventWriteError> {
        let (reply, receiver) = oneshot::channel();
        match self.queue.admit(WriteRequest::NodeEventTreeDelete(
            NodeEventTreeDeleteRequest { tree_id, reply },
        )) {
            Ok(()) => {}
            Err(AdmitRejection::Capacity) => return Err(NodeEventWriteError::StorageUnavailable),
            Err(AdmitRejection::Closed) => return Err(NodeEventWriteError::OutcomeUnknown),
        }
        receiver
            .await
            .map_err(|_| NodeEventWriteError::OutcomeUnknown)?
    }

    /// Runs gateway-local indexed reads on the store's fixed reader pool.
    ///
    /// This is intentionally not a general SQL port: only adaptors in this
    /// crate can provide the closure, so usecase/domain layers cannot acquire
    /// a connection or create a per-request runtime.
    pub(crate) fn submit_indexed_query_blocking<T, F>(
        &self,
        run: F,
    ) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        self.readers.submit_blocking(run)
    }
}

impl crate::usecase::application_lifecycle::operation::RecoveryResultCanonicalizer
    for LocalEventStore
{
    fn canonicalize_recovery_result(
        &self,
        outcome: crate::domain::local_event::RecoveryResultOutcomeRecord,
        classification: crate::domain::local_event::RecoveryResultClassification,
        resource_revision: u64,
        resource_view: crate::domain::local_event::RecoveryResourceViewRecord,
    ) -> Result<crate::domain::local_event::RecoveryResultRecord, ()> {
        super::state_record_codec::canonicalize_recovery_result_record(
            outcome,
            classification,
            resource_revision,
            resource_view,
        )
        .map_err(|_| ())
    }
}

impl crate::usecase::application_lifecycle::operation::OperationBindingAuthority
    for LocalEventStore
{
    fn mac(&self, message: &[u8]) -> [u8; 32] {
        crate::adaptor::gateway::local_event_store::hmac_sha256::hmac_sha256(
            &self.operation_binding_key,
            message,
        )
    }

    fn digest(&self, message: &[u8]) -> [u8; 32] {
        Sha256::digest(message).into()
    }

    fn seal_command(&self, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()> {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
        use ring::rand::{SecureRandom, SystemRandom};

        const MAGIC: &[u8; 5] = b"RLSA1";
        let key_bytes = crate::adaptor::gateway::local_event_store::hmac_sha256::hmac_sha256(
            &self.operation_binding_key,
            b"caller-attempt-command-aead/v1",
        );
        let key =
            LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key_bytes).map_err(|_| ())?);
        let mut nonce_bytes = [0u8; 12];
        SystemRandom::new().fill(&mut nonce_bytes).map_err(|_| ())?;
        let mut ciphertext = plaintext.to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(context),
            &mut ciphertext,
        )
        .map_err(|_| ())?;
        let mut envelope = Vec::with_capacity(MAGIC.len() + nonce_bytes.len() + ciphertext.len());
        envelope.extend_from_slice(MAGIC);
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    fn open_command(&self, context: &[u8], envelope: &[u8]) -> Result<Vec<u8>, ()> {
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};

        const MAGIC: &[u8; 5] = b"RLSA1";
        if envelope.len() < MAGIC.len() + 12 + CHACHA20_POLY1305.tag_len()
            || &envelope[..MAGIC.len()] != MAGIC
        {
            return Err(());
        }
        let key_bytes = crate::adaptor::gateway::local_event_store::hmac_sha256::hmac_sha256(
            &self.operation_binding_key,
            b"caller-attempt-command-aead/v1",
        );
        let key =
            LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key_bytes).map_err(|_| ())?);
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes.copy_from_slice(&envelope[MAGIC.len()..MAGIC.len() + 12]);
        let mut ciphertext = envelope[MAGIC.len() + 12..].to_vec();
        let plaintext = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(context),
                &mut ciphertext,
            )
            .map_err(|_| ())?;
        Ok(plaintext.to_vec())
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for LocalEventStore {
    fn canonical_mutation_identity_v1(
        &self,
        mutation: &LocalStateMutation,
    ) -> Result<Vec<u8>, String> {
        canonical_projection_mutation_identity_v1(mutation)
    }

    fn canonical_event_batch_identity_v1(
        &self,
        events: &[crate::domain::local_event::UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        crate::adaptor::gateway::local_event_store::envelope::canonical_event_batch_identity_v1(
            &self.registry,
            events,
        )
    }

    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        let identity = batch.commit_id.clone();
        let prepared = self.prepare(batch)?;
        let (reply, receiver) = oneshot::channel();
        match self
            .queue
            .admit(WriteRequest::Commit(CommitWriteRequest { prepared, reply }))
        {
            Ok(()) => {}
            Err(AdmitRejection::Capacity) => return Err(CommitBatchError::CapacityExceeded),
            Err(AdmitRejection::Closed) => {
                return Err(CommitBatchError::OutcomeUnknown { identity })
            }
        }
        match receiver.await {
            Ok(result) => result,
            // Reply loss after admission: the writer may or may not have
            // committed; resolve with the same commit identity only.
            Err(_) => Err(CommitBatchError::OutcomeUnknown { identity }),
        }
    }

    async fn resolve_commit(
        &self,
        identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        self.submit_query(move |connection| resolve_commit_row(connection, &identity))
            .await
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        let context = Arc::clone(&self.query_context);
        self.submit_query(move |connection| load_stream_page(connection, &context, &request))
            .await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        if matches!(
            &request,
            LocalEventQuery::PendingRecoveryPage { .. }
                | LocalEventQuery::PendingRecoverySnapshotPage { .. }
        ) {
            return self.recovery_snapshots.query(request).await;
        }
        let context = Arc::clone(&self.query_context);
        self.submit_query(move |connection| run_query(connection, &context, &request))
            .await
    }
}

impl Drop for LocalEventStore {
    fn drop(&mut self) {
        self.queue.close();
        self.readers.close();
        self.recovery_snapshots.close();
        for worker in self.reader_workers.drain(..) {
            let _ = worker.join();
        }
        if let Some(worker) = self.writer_worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod startup_error_classification_tests {
    use super::*;

    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    #[test]
    fn only_lock_contention_is_store_in_use() {
        assert_eq!(
            classify_writer_lock_error(&std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            LocalEventStoreOpenError::WriterLockHeld
        );
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::StorageFull,
            std::io::ErrorKind::Other,
        ] {
            assert_eq!(
                classify_writer_lock_error(&std::io::Error::from(kind)),
                LocalEventStoreOpenError::StorageUnavailable
            );
        }
    }

    #[test]
    fn sqlite_io_permission_and_capacity_failures_are_storage_unavailable() {
        for code in [
            rusqlite::ffi::SQLITE_PERM,
            rusqlite::ffi::SQLITE_BUSY,
            rusqlite::ffi::SQLITE_LOCKED,
            rusqlite::ffi::SQLITE_NOMEM,
            rusqlite::ffi::SQLITE_READONLY,
            rusqlite::ffi::SQLITE_IOERR,
            rusqlite::ffi::SQLITE_FULL,
            rusqlite::ffi::SQLITE_CANTOPEN,
            rusqlite::ffi::SQLITE_PROTOCOL,
            rusqlite::ffi::SQLITE_TOOBIG,
        ] {
            assert_eq!(
                classify_sqlite_error(
                    &sqlite_failure(code),
                    LocalEventStoreOpenError::SchemaEvolutionFailed,
                ),
                LocalEventStoreOpenError::StorageUnavailable
            );
        }
        assert_eq!(
            classify_sqlite_error(
                &sqlite_failure(rusqlite::ffi::SQLITE_CORRUPT),
                LocalEventStoreOpenError::StoreValidationFailed,
            ),
            LocalEventStoreOpenError::StoreValidationFailed
        );
        assert_eq!(
            classify_connection_error(
                &ConnectionError::SqliteTooOld { version_number: 0 },
                LocalEventStoreOpenError::StorageUnavailable,
            ),
            LocalEventStoreOpenError::UnsupportedRuntime
        );
    }

    #[test]
    fn startup_maintenance_reopen_failures_use_the_connection_classifier() {
        assert_eq!(
            classify_startup_maintenance_error(&StartupMaintenanceError::Connection(
                ConnectionError::Sqlite(sqlite_failure(rusqlite::ffi::SQLITE_IOERR)),
            )),
            LocalEventStoreOpenError::StorageUnavailable
        );
        assert_eq!(
            classify_startup_maintenance_error(&StartupMaintenanceError::Connection(
                ConnectionError::SqliteTooOld { version_number: 0 },
            )),
            LocalEventStoreOpenError::UnsupportedRuntime
        );
    }
}
