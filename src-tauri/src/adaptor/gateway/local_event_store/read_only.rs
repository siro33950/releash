//! Read-only local-event repository for CLI processes that may run while the
//! desktop process owns the single SQLite writer lock.
//!
//! Opening this adapter never creates or evolves the store. Its mutation
//! entry point always fails closed; reads validate the fixed SQLite store
//! before each snapshot.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::clock::SystemStoreClock;
use crate::adaptor::gateway::local_event_store::connection::open_reader;
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
    GlobalSequence, LoadStreamRequest, LocalAtomicBatch, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventSignal, LocalEventSubscription,
    LocalEventTransactionRepository, LocalStateMutation, SafeOperationFailure,
    SessionOperationFailureKind,
};
use futures_util::stream;

const STORE_NOT_READY: &str = "the fixed local event store is not ready";

pub(crate) struct LocalEventReadStore {
    database_path: PathBuf,
    database_identity: DatabaseFileIdentity,
    installation_id: String,
    query_context: Arc<QueryContext>,
    readers: Arc<ReaderPool>,
    reader_workers: Vec<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DatabaseFileIdentity {
    stable: Option<StableFileId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StableFileId {
    volume: u64,
    index: u128,
}

impl DatabaseFileIdentity {
    fn read(path: &Path) -> Result<Self, LocalEventQueryError> {
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
            true,
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
    pub(crate) fn open(app_data_root: &Path) -> Result<Arc<Self>, String> {
        let layout = StoreLayout::new(app_data_root);
        let database_path = layout.database_path();
        if !database_path.try_exists().map_err(|_| STORE_NOT_READY)? {
            return Err(STORE_NOT_READY.to_string());
        }
        let connection = open_reader(&database_path)
            .map_err(|error| format!("failed to open canonical local event reader: {error}"))?;
        let database_identity =
            DatabaseFileIdentity::read(&database_path).map_err(|_| STORE_NOT_READY)?;
        validate_current_schema(&connection).map_err(|_| STORE_NOT_READY.to_string())?;
        let metadata: (String, Vec<u8>, String) = connection
            .query_row(
                "SELECT installation_id, cursor_hmac_key, process_instance_id
                 FROM store_metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| STORE_NOT_READY.to_string())?;
        let clock: Arc<dyn crate::adaptor::gateway::local_event_store::clock::StoreClock> =
            Arc::new(SystemStoreClock);
        let query_context = Arc::new(QueryContext {
            registry: Arc::new(EventCodecRegistry::new()),
            cursor_key: metadata.1,
            process_instance_id: metadata.2,
            clock: Arc::clone(&clock),
        });
        let readers = ReaderPool::new(clock);
        let mut connections = Vec::with_capacity(READER_POOL_SIZE);
        connections.push(connection);
        for _ in 1..READER_POOL_SIZE {
            connections.push(open_reader(&database_path).map_err(|_| STORE_NOT_READY)?);
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
            installation_id: metadata.0,
            query_context,
            readers,
            reader_workers,
        }))
    }

    pub(crate) fn installation_id(&self) -> &str {
        &self.installation_id
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
        let receiver = self.readers.submit(move |connection| {
            validate_reader_snapshot(
                connection,
                &database_path,
                database_identity,
                &installation_id,
            )?;
            operation(connection, &query_context)
        })?;
        receiver
            .await
            .map_err(|_| LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "local event read store reader reply lost",
                    uuid::Uuid::new_v4().to_string(),
                ),
            })?
    }

    pub(crate) fn submit_indexed_query_blocking<T, F>(
        &self,
        operation: F,
    ) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        let database_path = self.database_path.clone();
        let database_identity = self.database_identity;
        let installation_id = self.installation_id.clone();
        self.readers.submit_blocking(move |connection| {
            validate_reader_snapshot(
                connection,
                &database_path,
                database_identity,
                &installation_id,
            )?;
            operation(connection)
        })
    }
}

fn validate_reader_snapshot(
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
        Err(CommitBatchError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::PersistFailure,
                false,
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
        Err(LocalEventQueryError::StorageUnavailable {
            failure: SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "Commit resolution requires the canonical writer authority.",
                uuid::Uuid::new_v4().to_string(),
            ),
        })
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
        if matches!(&request, LocalEventQuery::CommitByIdentity { .. }) {
            return Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "Commit resolution requires the canonical writer authority.",
                    uuid::Uuid::new_v4().to_string(),
                ),
            });
        }
        self.read(move |connection, context| run_query(connection, context, &request))
            .await
    }

    fn query_blocking(
        &self,
        request: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        if matches!(&request, LocalEventQuery::CommitByIdentity { .. }) {
            return Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "Commit resolution requires the canonical writer authority.",
                    uuid::Uuid::new_v4().to_string(),
                ),
            });
        }
        let context = Arc::clone(&self.query_context);
        self.submit_indexed_query_blocking(move |connection| {
            run_query(connection, &context, &request)
        })
    }

    fn subscribe(&self, _after: GlobalSequence) -> LocalEventSubscription {
        LocalEventSubscription::new(Box::pin(stream::once(async {
            LocalEventSignal::ReplayRequired
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::connection::open_writer;
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::domain::local_event::{
        CommitOperationKind, IdempotencyBinding, LocalEventQueryResult,
    };

    #[test]
    fn unrelated_files_are_never_a_read_fallback_without_sqlite_authority() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        std::fs::create_dir_all(root.path().join("sessions/session-legacy"))
            .expect("legacy session fixture");

        let error = match LocalEventReadStore::open(root.path()) {
            Ok(_) => panic!("unrelated files must not become a cross-process read authority"),
            Err(error) => error,
        };

        assert_eq!(error, STORE_NOT_READY);
    }

    #[test]
    fn sqlite_authority_requires_current_schema() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        drop(writer);

        let database_path = StoreLayout::new(root.path()).database_path();
        let connection = open_writer(&database_path).expect("maintenance connection");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("stale schema fixture");
        drop(connection);

        let error = match LocalEventReadStore::open(root.path()) {
            Ok(_) => panic!("stale schema must not publish canonical session state"),
            Err(error) => error,
        };
        assert_eq!(error, STORE_NOT_READY);
    }

    #[test]
    fn stable_file_identity_comparison_detects_only_known_mismatches() {
        let first = DatabaseFileIdentity {
            stable: Some(StableFileId {
                volume: 1,
                index: 2,
            }),
        };
        let same = DatabaseFileIdentity {
            stable: Some(StableFileId {
                volume: 1,
                index: 2,
            }),
        };
        let replaced = DatabaseFileIdentity {
            stable: Some(StableFileId {
                volume: 1,
                index: 3,
            }),
        };
        let unavailable = DatabaseFileIdentity { stable: None };

        assert!(!database_identity_changed(first, same));
        assert!(database_identity_changed(first, replaced));
        assert!(!database_identity_changed(first, unavailable));
        assert!(!database_identity_changed(unavailable, replaced));
    }

    #[test]
    fn reader_snapshot_validation_requires_only_markers_and_installation_identity() {
        let root = tempfile::TempDir::new().expect("lightweight reader validation");
        let database_path = root.path().join("marker-only.sqlite3");
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .pragma_update(None, "application_id", super::super::schema::APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(
                None,
                "user_version",
                super::super::schema::CURRENT_SCHEMA_VERSION,
            )
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE store_metadata (
                     id INTEGER PRIMARY KEY,
                     installation_id TEXT NOT NULL
                 );
                 INSERT INTO store_metadata (id, installation_id)
                 VALUES (1, 'marker-installation');",
            )
            .unwrap();
        let identity = DatabaseFileIdentity::read(&database_path).unwrap();

        assert!(validate_reader_snapshot(
            &connection,
            &database_path,
            identity,
            "marker-installation",
        )
        .is_ok());
        assert!(validate_current_schema(&connection).is_err());
    }

    #[tokio::test]
    async fn writer_commit_and_wal_checkpoint_preserve_database_file_identity() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        let reader = LocalEventReadStore::open(root.path()).expect("concurrent canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let before = DatabaseFileIdentity::read(&database_path).unwrap();
        writer
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse("read-only-identity-commit").unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: writer.installation_id().to_string(),
                    operation_kind: CommitOperationKind::Projection,
                    idempotency_key: "read-only-identity-commit".to_string(),
                    payload_hash: [21; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: Vec::new(),
            })
            .await
            .expect("normal writer commit");
        let maintenance = open_writer(&database_path).expect("checkpoint connection");
        maintenance
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("truncate WAL checkpoint");
        drop(maintenance);

        let after = DatabaseFileIdentity::read(&database_path).unwrap();
        assert_eq!(after, before);
        let result = reader
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: "missing-after-checkpoint".to_string(),
            })
            .await;
        assert!(matches!(
            result,
            Ok(LocalEventQueryResult::SessionProjectionByIdentity(None))
        ));
    }

    #[tokio::test]
    async fn reader_allows_bounded_queries_but_fails_mutation_and_resolution_closed() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        let reader = LocalEventReadStore::open(root.path()).expect("concurrent canonical reader");

        let query = reader
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: "missing-session".to_string(),
            })
            .await
            .expect("bounded point query");
        assert!(matches!(
            query,
            LocalEventQueryResult::SessionProjectionByIdentity(None)
        ));

        let commit_id = CommitIdentity::parse("read-only-commit").expect("commit identity");
        let batch = LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: reader.installation_id().to_string(),
                operation_kind: CommitOperationKind::Projection,
                idempotency_key: "read-only-key".to_string(),
                payload_hash: [0; 32],
            },
            expected_heads: Vec::new(),
            events: Vec::new(),
            state_mutations: Vec::new(),
        };
        let commit_error = reader
            .commit_batch(batch)
            .await
            .expect_err("read-only repository must reject commit_batch");
        assert!(matches!(
            commit_error,
            CommitBatchError::StorageUnavailable { failure }
                if failure.kind == SessionOperationFailureKind::PersistFailure
                    && !failure.retryable
        ));
        assert_eq!(
            writer
                .resolve_commit(commit_id.clone())
                .await
                .expect("writer proves absence"),
            CommitResolution::NotCommitted
        );

        let resolve_error = reader
            .resolve_commit(commit_id)
            .await
            .expect_err("read-only repository cannot prove non-commit");
        assert!(matches!(
            resolve_error,
            LocalEventQueryError::StorageUnavailable { failure }
                if failure.kind == SessionOperationFailureKind::OutcomeUnknown
                    && failure.retryable
        ));
    }

    #[tokio::test]
    async fn reader_fails_closed_when_schema_changes_after_open() {
        // Given
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(root.path()).expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let maintenance = open_writer(&database_path).expect("maintenance connection");
        maintenance
            .pragma_update(None, "user_version", 2)
            .expect("replace schema marker");
        drop(maintenance);

        // When
        let error = reader
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: "missing-session".to_string(),
            })
            .await
            .expect_err("stale schema must fail closed on every read");

        // Then
        assert!(matches!(error, LocalEventQueryError::Corrupt { .. }));
    }

    #[tokio::test]
    async fn reader_fails_closed_when_installation_changes_after_open() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(root.path()).expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let maintenance = open_writer(&database_path).expect("maintenance connection");
        maintenance
            .execute(
                "UPDATE store_metadata SET installation_id = ?1 WHERE id = 1",
                [uuid::Uuid::new_v4().to_string()],
            )
            .expect("replace installation identity");
        drop(maintenance);

        let error = reader
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: "missing-session".to_string(),
            })
            .await
            .expect_err("installation replacement must fail closed on every read");

        assert!(matches!(error, LocalEventQueryError::Corrupt { .. }));
    }

    #[tokio::test]
    async fn reader_fails_closed_when_database_file_is_replaced_after_open() {
        // Given
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(root.path()).expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let replaced_path = root.path().join("replaced-local-event-store.sqlite3");
        std::fs::rename(&database_path, &replaced_path).expect("retain replaced fixture");
        let replacement =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("replacement authority");

        // When
        let error = reader
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: "missing-session".to_string(),
            })
            .await
            .expect_err("replaced database must fail closed on every read");

        // Then
        assert!(matches!(error, LocalEventQueryError::Corrupt { .. }));
        drop(replacement);
    }
}
