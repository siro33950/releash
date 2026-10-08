//! Read-only local-event repository for CLI processes that may run while the
//! desktop process owns the single SQLite writer lock.
//!
//! Opening this adapter never creates or evolves the store. Its mutation
//! entry point always fails closed; reads validate the fixed SQLite store
//! before each snapshot.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
use crate::adaptor::gateway::local_event_store::projection_record_codec::canonical_mutation_identity_v1 as canonical_projection_mutation_identity_v1;
use crate::adaptor::gateway::local_event_store::reader::{
    load_stream_page, run_query, QueryContext, ReaderPool, READER_POOL_SIZE,
};
use crate::adaptor::gateway::local_event_store::schema::{
    validate_current_schema, validate_current_schema_marker,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    LoadStreamRequest, LocalAtomicBatch, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
    SafeOperationFailure, SessionOperationFailureKind,
};
use crate::infrastructure::local_event_store_connection::open_reader;

pub const STORE_NOT_READY: &str = "the fixed local event store is not ready";

pub struct LocalEventReadStore {
    database_path: PathBuf,
    database_identity: DatabaseFileIdentity,
    installation_id: String,
    query_context: Arc<QueryContext>,
    readers: Arc<ReaderPool>,
    reader_workers: Vec<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseFileIdentity {
    stable: Option<StableFileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StableFileId {
    volume: u64,
    index: u128,
}

impl DatabaseFileIdentity {
    pub fn read(path: &Path) -> Result<Self, LocalEventQueryError> {
        Ok(Self {
            stable: read_stable_file_id(path)?,
        })
    }
}

fn database_identity_changed(
    expected: DatabaseFileIdentity,
    current: DatabaseFileIdentity,
) -> bool {
    matches!(
        (expected.stable, current.stable),
        (Some(expected), Some(current)) if expected != current
    )
}

fn database_metadata_unavailable() -> LocalEventQueryError {
    LocalEventQueryError::StorageUnavailable {
        failure: SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            crate::domain::failure::TechnicalFailureNature::Transient,
            "local event read store database metadata is unavailable",
            uuid::Uuid::new_v4().to_string(),
        ),
    }
}

#[cfg(unix)]
fn read_stable_file_id(path: &Path) -> Result<Option<StableFileId>, LocalEventQueryError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(path).map_err(|_| database_metadata_unavailable())?;
    Ok(Some(StableFileId {
        volume: metadata.dev(),
        index: u128::from(metadata.ino()),
    }))
}

#[cfg(windows)]
fn read_stable_file_id(path: &Path) -> Result<Option<StableFileId>, LocalEventQueryError> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let file = std::fs::File::open(path).map_err(|_| database_metadata_unavailable())?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a valid handle for the duration of the call and
    // `information` points to writable storage of the required type.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Ok(None);
    }
    Ok(Some(StableFileId {
        volume: u64::from(information.dwVolumeSerialNumber),
        index: (u128::from(information.nFileIndexHigh) << 32)
            | u128::from(information.nFileIndexLow),
    }))
}

#[cfg(not(any(unix, windows)))]
fn read_stable_file_id(_path: &Path) -> Result<Option<StableFileId>, LocalEventQueryError> {
    Ok(None)
}

impl LocalEventReadStore {
    pub fn open(
        app_data_root: &Path,
        limiter: Arc<crate::common::retry::RetryLimiter>,
    ) -> Result<Arc<Self>, String> {
        let layout = StoreLayout::new(app_data_root);
        let database_path = layout.database_path();
        if !database_path.try_exists().map_err(|_| STORE_NOT_READY)? {
            return Err(STORE_NOT_READY.to_string());
        }
        let connection = open_reader(&database_path, limiter.clone())
            .map_err(|error| format!("failed to open canonical local event reader: {error}"))?;
        let database_identity =
            DatabaseFileIdentity::read(&database_path).map_err(|_| STORE_NOT_READY)?;
        validate_current_schema(&connection).map_err(|_| STORE_NOT_READY.to_string())?;
        let installation_id: String = connection
            .query_row(
                "SELECT installation_id
                 FROM store_metadata WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| STORE_NOT_READY.to_string())?;
        let query_context = Arc::new(QueryContext {
            registry: Arc::new(EventCodecRegistry::new()),
        });
        let readers = ReaderPool::new();
        let mut connections = Vec::with_capacity(READER_POOL_SIZE);
        connections.push(connection);
        for _ in 1..READER_POOL_SIZE {
            connections
                .push(open_reader(&database_path, limiter.clone()).map_err(|_| STORE_NOT_READY)?);
        }
        let mut reader_workers: Vec<std::thread::JoinHandle<()>> =
            Vec::with_capacity(READER_POOL_SIZE);
        for (index, connection) in connections.into_iter().enumerate() {
            let worker_readers = Arc::clone(&readers);
            let worker = match std::thread::Builder::new()
                .name(format!("local-event-read-store-reader-{index}"))
                .spawn(move || worker_readers.run_worker(connection))
            {
                Ok(worker) => worker,
                Err(_) => {
                    readers.close();
                    for worker in reader_workers {
                        let _ = worker.join();
                    }
                    return Err(STORE_NOT_READY.to_string());
                }
            };
            reader_workers.push(worker);
        }
        Ok(Arc::new(Self {
            database_path,
            database_identity,
            installation_id,
            query_context,
            readers,
            reader_workers,
        }))
    }

    async fn read<T, F>(&self, operation: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection, &QueryContext) -> Result<T, LocalEventQueryError>
            + Send
            + 'static,
    {
        let query_context = Arc::clone(&self.query_context);
        let database_path = self.database_path.clone();
        let database_identity = self.database_identity;
        let installation_id = self.installation_id.clone();
        self.readers
            .submit(move |connection| {
                validate_reader_snapshot(
                    connection,
                    &database_path,
                    database_identity,
                    &installation_id,
                )?;
                operation(connection, &query_context)
            })
            .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub(crate) async fn submit_query<T, F>(&self, operation: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        self.read(move |connection, _| operation(connection)).await
    }
}

pub fn validate_reader_snapshot(
    connection: &rusqlite::Connection,
    database_path: &Path,
    expected_identity: DatabaseFileIdentity,
    expected_installation_id: &str,
) -> Result<(), LocalEventQueryError> {
    let correlation_id = || uuid::Uuid::new_v4().to_string();
    if database_identity_changed(
        expected_identity,
        DatabaseFileIdentity::read(database_path)?,
    ) {
        let correlation_id = correlation_id();
        log::error!("read-only local event store database identity changed [{correlation_id}]");
        return Err(LocalEventQueryError::Corrupt { correlation_id });
    }
    validate_current_schema_marker(connection).map_err(|error| {
        let correlation_id = correlation_id();
        log::error!(
            "read-only local event store schema validation failed [{correlation_id}]: {error}"
        );
        LocalEventQueryError::Corrupt { correlation_id }
    })?;
    let installation_id = connection
        .query_row(
            "SELECT installation_id FROM store_metadata WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| {
            let correlation_id = correlation_id();
            log::error!(
                "read-only local event store identity lookup failed [{correlation_id}]: {error}"
            );
            LocalEventQueryError::Corrupt { correlation_id }
        })?;
    if installation_id != expected_installation_id {
        let correlation_id = correlation_id();
        log::error!("read-only local event store installation changed [{correlation_id}]");
        return Err(LocalEventQueryError::Corrupt { correlation_id });
    }
    Ok(())
}

impl Drop for LocalEventReadStore {
    fn drop(&mut self) {
        self.readers.close();
        for worker in self.reader_workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for LocalEventReadStore {
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
            &EventCodecRegistry::new(),
            events,
        )
    }

    async fn commit_batch(
        &self,
        _batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        Err(CommitBatchError::StorageAccessRequired {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                crate::domain::failure::TechnicalFailureNature::Other,
                "The CLI session reader cannot accept mutations.",
                uuid::Uuid::new_v4().to_string(),
            ),
        })
    }

    async fn resolve_commit(
        &self,
        _identity: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        // A concurrent read-only snapshot cannot prove non-commit. Only the
        // exclusive writer owner may resolve an OutcomeUnknown identity.
        Err(LocalEventQueryError::CanonicalWriterRequired)
    }

    async fn load_stream(
        &self,
        request: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.read(move |connection, context| load_stream_page(connection, context, &request))
            .await
    }

    async fn query(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        self.read(move |connection, _| run_query(connection, &request))
            .await
    }
}

#[cfg(test)]
#[path = "read_only_test.rs"]
mod read_only_tests;
