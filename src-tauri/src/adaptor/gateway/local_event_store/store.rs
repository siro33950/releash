//! `LocalEventStore`: the SQLite implementation of
//! `LocalEventTransactionRepository`, the single mutation authority.
//!
//! One dedicated writer thread and up to four dedicated reader threads own
//! every rusqlite call. The async trait methods only validate, encode, and
//! exchange messages with those threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use futures_util::stream;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot};

use crate::adaptor::gateway::local_event_store::authority::{
    cas_authority, read_authority, AuthorityError, LocalStoreAuthorityPointerV1, StoreLayout,
};
use crate::adaptor::gateway::local_event_store::clock::{StoreClock, SystemStoreClock};
use crate::adaptor::gateway::local_event_store::commit::{
    cleanup_compacted_shutdown_details, execute_commit, resolve_commit_row,
};
use crate::adaptor::gateway::local_event_store::connection::{
    check_sqlite_version, open_reader, open_writer, set_owner_only_permissions, ConnectionError,
};
use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
use crate::adaptor::gateway::local_event_store::migration::{
    activated_migration_inventory_hash, import_legacy, mark_activating, prepare_authority,
    verify_activated_migration, verify_source_unchanged,
};
use crate::adaptor::gateway::local_event_store::projection_record_codec::canonical_mutation_identity_v1 as canonical_projection_mutation_identity_v1;
use crate::adaptor::gateway::local_event_store::reader::{
    load_stream_page, run_query, QueryContext, ReaderPool, RecoverySnapshotPager, READER_POOL_SIZE,
};
use crate::adaptor::gateway::local_event_store::schema::apply_schema;
use crate::adaptor::gateway::local_event_store::writer::{
    AdmitRejection, PreparedBatch, PreparedEvent, QueuePop, WriteQueue, WriteRequest,
    MAX_BATCH_DECODED_BYTES, MAX_BATCH_EVENTS, MAX_BATCH_STATE_MUTATIONS,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    GlobalSequence, LoadStreamRequest, LocalAtomicBatch, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventSignal, LocalEventSubscription,
    LocalEventTransactionRepository, LocalStateMutation, OperationReceiptRecord,
    OperationRecordMutation, SafeOperationFailure, SessionOperationFailureKind,
};

fn migration_quit_operation_matches(
    operation: &OperationRecordMutation,
    migration_id: &str,
) -> bool {
    matches!(
        &operation.receipt,
        OperationReceiptRecord::MigrationApplicationQuit {
            migration_id: saved,
            ..
        } if saved == migration_id
    ) && operation.latest_status.migration_quit
        && operation.latest_status.kind
            == crate::domain::local_event::OperationKind::ApplicationQuit
}

fn correlation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn random_key_32() -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key
}

#[derive(Debug)]
pub enum LocalEventStoreOpenError {
    /// The authority pointer is `Legacy`; migration (T-06) must run before
    /// this store can open for normal admission.
    LegacyAuthorityActive,
    WriterLockHeld,
    Authority(AuthorityError),
    Connection(ConnectionError),
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Corrupt {
        reason: String,
    },
}

impl std::fmt::Display for LocalEventStoreOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LegacyAuthorityActive => {
                write!(f, "authority pointer is Legacy; migration required")
            }
            Self::WriterLockHeld => write!(f, "another process holds the writer lock"),
            Self::Authority(inner) => write!(f, "{inner}"),
            Self::Connection(inner) => write!(f, "{inner}"),
            Self::Io(inner) => write!(f, "{inner}"),
            Self::Sqlite(inner) => write!(f, "{inner}"),
            Self::Corrupt { reason } => write!(f, "store corrupt: {reason}"),
        }
    }
}

impl std::error::Error for LocalEventStoreOpenError {}

impl From<AuthorityError> for LocalEventStoreOpenError {
    fn from(inner: AuthorityError) -> Self {
        Self::Authority(inner)
    }
}
impl From<ConnectionError> for LocalEventStoreOpenError {
    fn from(inner: ConnectionError) -> Self {
        Self::Connection(inner)
    }
}
impl From<std::io::Error> for LocalEventStoreOpenError {
    fn from(inner: std::io::Error) -> Self {
        Self::Io(inner)
    }
}
impl From<rusqlite::Error> for LocalEventStoreOpenError {
    fn from(inner: rusqlite::Error) -> Self {
        Self::Sqlite(inner)
    }
}

pub struct LocalEventStoreConfig {
    pub app_data_root: PathBuf,
    pub clock: Arc<dyn StoreClock>,
    pub registry: Arc<EventCodecRegistry>,
    pub fault: Arc<FaultInjector>,
}

impl LocalEventStoreConfig {
    /// Production configuration: system clock, default registry, no faults.
    pub fn production(app_data_root: PathBuf) -> Self {
        Self {
            app_data_root,
            clock: Arc::new(SystemStoreClock),
            registry: Arc::new(EventCodecRegistry::new()),
            fault: Arc::new(FaultInjector::new()),
        }
    }
}

struct BroadcastSignal {
    commit_id: String,
    max_global_sequence: i64,
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
    notifications: broadcast::Sender<Arc<BroadcastSignal>>,
    generation_id: String,
    operation_binding_key: [u8; 32],
    migration_admission: Arc<AtomicU8>,
    active_migration_id: Option<String>,
    // Held for the lifetime of the store: exclusive app-data writer lock.
    _writer_lock: std::fs::File,
}

impl LocalEventStore {
    /// Open (or bootstrap) the store under `app_data_root`.
    ///
    /// New installations create an empty SQLite generation and publish the
    /// `Sqlite` authority pointer. A `Legacy` pointer fails closed until the
    /// migration task performs the cutover.
    pub fn open(config: LocalEventStoreConfig) -> Result<Arc<Self>, LocalEventStoreOpenError> {
        check_sqlite_version()?;
        let layout = StoreLayout::new(&config.app_data_root);
        layout.ensure_directories()?;

        // Exclusive writer lock: absence of a commit row is only proof of
        // non-commit while this lock is held and WAL recovery has finished.
        let lock_path = layout.store_directory().join("writer.lock");
        let writer_lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        set_owner_only_permissions(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&writer_lock)
            .map_err(|_| LocalEventStoreOpenError::WriterLockHeld)?;

        let authority =
            prepare_authority(&config.app_data_root, &layout, read_authority(&layout)?)?;
        let (generation_id, expected_authority) = match authority {
            None => {
                let generation_id = uuid::Uuid::new_v4().to_string();
                (generation_id, None)
            }
            Some(pointer @ LocalStoreAuthorityPointerV1::Sqlite { .. }) => {
                let LocalStoreAuthorityPointerV1::Sqlite { generation_id, .. } = &pointer else {
                    unreachable!("matched Sqlite variant above");
                };
                (generation_id.clone(), Some(pointer))
            }
            Some(
                pointer @ LocalStoreAuthorityPointerV1::Legacy {
                    migration: Some(_), ..
                },
            ) => {
                let LocalStoreAuthorityPointerV1::Legacy {
                    migration: Some(migration),
                    ..
                } = &pointer
                else {
                    unreachable!()
                };
                (migration.staging_generation_id.clone(), Some(pointer))
            }
            Some(LocalStoreAuthorityPointerV1::Legacy {
                migration: None, ..
            }) => return Err(LocalEventStoreOpenError::LegacyAuthorityActive),
        };

        let database_path = layout.generation_database_path(&generation_id);
        let mut writer_connection = open_writer(&database_path)?;
        set_owner_only_permissions(&database_path)?;
        apply_schema(&writer_connection)?;

        let boot_id = uuid::Uuid::new_v4().to_string();
        let now_ms = config.clock.now_ms().max(0);
        let mut pending_migration = None;
        let mut verified_activated_migration_id = None;
        let store_id = match &expected_authority {
            None => {
                let store_id = uuid::Uuid::new_v4().to_string();
                let cursor_key = random_key_32();
                let operation_binding_key = random_key_32();
                writer_connection.execute(
                    "INSERT INTO store_metadata (
                        id, schema_version, store_id, generation_id, created_at_ms,
                        cursor_hmac_key, operation_binding_hmac_key, boot_id,
                        next_global_sequence, health,
                        current_shutdown_plan_id, current_shutdown_epoch,
                        shutdown_pointer_revision
                     ) VALUES (1, 1, ?1, ?2, ?3, ?4, ?5, ?6, 1, 'ok', NULL, NULL, 0)",
                    rusqlite::params![
                        store_id,
                        generation_id,
                        now_ms,
                        cursor_key.as_slice(),
                        operation_binding_key.as_slice(),
                        boot_id
                    ],
                )?;
                let pointer = LocalStoreAuthorityPointerV1::Sqlite {
                    generation_id: generation_id.clone(),
                    store_id: store_id.clone(),
                    activated_migration_id: None,
                };
                cas_authority(&layout, None, &pointer, None)?;
                store_id
            }
            Some(LocalStoreAuthorityPointerV1::Sqlite {
                store_id: pointer_store_id,
                activated_migration_id,
                ..
            }) => {
                let (stored, stored_generation): (String, String) = writer_connection.query_row(
                    "SELECT store_id, generation_id FROM store_metadata WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if &stored != pointer_store_id {
                    return Err(LocalEventStoreOpenError::Corrupt {
                        reason: "store_metadata store_id does not match authority pointer"
                            .to_string(),
                    });
                }
                if stored_generation != generation_id {
                    return Err(LocalEventStoreOpenError::Corrupt {
                        reason: "store_metadata generation_id does not match authority pointer"
                            .to_string(),
                    });
                }
                if let Some(migration_id) = activated_migration_id {
                    verify_activated_migration(&writer_connection, migration_id).map_err(
                        |error| LocalEventStoreOpenError::Corrupt {
                            reason: format!(
                                "activated migration parity does not match authority pointer: {error}"
                            ),
                        },
                    )?;
                    verified_activated_migration_id = Some(migration_id.clone());
                }
                writer_connection.execute(
                    "UPDATE store_metadata SET boot_id = ?1 WHERE id = 1",
                    rusqlite::params![boot_id],
                )?;
                stored
            }
            Some(
                pointer @ LocalStoreAuthorityPointerV1::Legacy {
                    migration: Some(migration),
                    ..
                },
            ) => {
                let candidate_store_id = uuid::Uuid::new_v4().to_string();
                let cursor_key = random_key_32();
                let operation_binding_key = random_key_32();
                writer_connection.execute(
                    "INSERT OR IGNORE INTO store_metadata (
                        id, schema_version, store_id, generation_id, created_at_ms,
                        cursor_hmac_key, operation_binding_hmac_key, boot_id,
                        next_global_sequence, health, current_shutdown_plan_id,
                        current_shutdown_epoch, shutdown_pointer_revision
                     ) VALUES (1, 1, ?1, ?2, ?3, ?4, ?5, ?6, 1,
                               'recovering', NULL, NULL, 0)",
                    rusqlite::params![
                        candidate_store_id,
                        generation_id,
                        now_ms,
                        cursor_key.as_slice(),
                        operation_binding_key.as_slice(),
                        boot_id,
                    ],
                )?;
                let (stored, stored_generation): (String, String) = writer_connection.query_row(
                    "SELECT store_id, generation_id FROM store_metadata WHERE id = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if stored_generation != generation_id {
                    return Err(LocalEventStoreOpenError::Corrupt {
                        reason: "staging store generation_id does not match migration locator"
                            .to_string(),
                    });
                }
                pending_migration = Some((pointer.clone(), migration.migration_id.clone()));
                stored
            }
            Some(LocalStoreAuthorityPointerV1::Legacy {
                migration: None, ..
            }) => unreachable!("legacy migration locator prepared above"),
        };
        let cursor_key: Vec<u8> = writer_connection.query_row(
            "SELECT cursor_hmac_key FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let operation_binding_key: Vec<u8> = writer_connection.query_row(
            "SELECT operation_binding_hmac_key FROM store_metadata WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let operation_binding_key: [u8; 32] =
            operation_binding_key
                .try_into()
                .map_err(|_| LocalEventStoreOpenError::Corrupt {
                    reason: "operation binding key has an invalid length".to_string(),
                })?;

        let active_migration_id = pending_migration
            .as_ref()
            .map(|(_, migration_id)| migration_id.clone())
            .or(verified_activated_migration_id.clone());
        let queue = WriteQueue::new();
        // 0=migrating, 1=normal admission, 2=blocked, 3=cut over but waiting
        // for application read authorities to install. Queries remain
        // available in every state so the application can supervise progress.
        let migration_admission = Arc::new(AtomicU8::new(if pending_migration.is_some() {
            0
        } else if verified_activated_migration_id.is_some() {
            3
        } else {
            1
        }));
        let (notifications, _) = broadcast::channel(1024);
        let readers = ReaderPool::new(Arc::clone(&config.clock));
        let query_context = Arc::new(QueryContext {
            registry: Arc::clone(&config.registry),
            cursor_key,
            boot_id: boot_id.clone(),
            clock: Arc::clone(&config.clock),
        });
        let recovery_snapshots =
            RecoverySnapshotPager::new(database_path.clone(), Arc::clone(&query_context));

        // Writer worker thread: sole owner of the writer connection.
        {
            let queue = Arc::clone(&queue);
            let fault = Arc::clone(&config.fault);
            let clock = Arc::clone(&config.clock);
            let notifications = notifications.clone();
            let migration_admission_worker = Arc::clone(&migration_admission);
            let migration_root = config.app_data_root.clone();
            let migration_layout = layout.clone();
            let migration_database_path = database_path.clone();
            let migration_generation_id = generation_id.clone();
            let migration_store_id = store_id.clone();
            let migration_boot_id = boot_id.clone();
            std::thread::Builder::new()
                .name("local-event-store-writer".to_string())
                .spawn(move || {
                    if let Some((legacy_pointer, migration_id)) = pending_migration {
                        let migration_result = (|| -> Result<(), String> {
                            let mut drain_critical = |connection: &rusqlite::Connection| {
                                while let Some(request) = queue.try_pop_critical() {
                                    let result = execute_commit(
                                        connection,
                                        &request.prepared,
                                        clock.now_ms().max(0),
                                        &fault,
                                    );
                                    if let Ok(CommitBatchResult::Committed(batch)) = &result {
                                        if let Some((_, last)) = batch.sequence_range {
                                            let _ = notifications.send(Arc::new(
                                                BroadcastSignal {
                                                    commit_id: batch
                                                        .commit_id
                                                        .as_str()
                                                        .to_string(),
                                                    max_global_sequence: last.value(),
                                                },
                                            ));
                                        }
                                    }
                                    let _ = request.reply.send(result);
                                }
                            };
                            let _inventory_hash = if let Some(inventory_hash) =
                                activated_migration_inventory_hash(
                                    &writer_connection,
                                    &migration_id,
                                )
                                .map_err(|error| error.to_string())?
                            {
                                // Activation committed before a prior process
                                // stopped. The sealed proof is the only legal
                                // resume input; do not consult or re-import the
                                // legacy source at this pointer-CAS boundary.
                                verify_activated_migration(&writer_connection, &migration_id)
                                    .map_err(|error| error.to_string())?;
                                inventory_hash
                            } else {
                                let inventory_hash = import_legacy(
                                    &mut writer_connection,
                                    &migration_root,
                                    &migration_id,
                                    &migration_generation_id,
                                    now_ms,
                                    &mut drain_critical,
                                )
                                .map_err(|error| error.to_string())?;
                                verify_source_unchanged(&migration_root, inventory_hash, || {
                                    drain_critical(&writer_connection)
                                })
                                .map_err(|error| error.to_string())?;
                                let integrity: String = writer_connection
                                    .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                                    .map_err(|error| error.to_string())?;
                                if integrity != "ok" {
                                    return Err(
                                        "staging SQLite integrity check failed".to_string()
                                    );
                                }
                                mark_activating(
                                    &mut writer_connection,
                                    &migration_id,
                                    &migration_boot_id,
                                    inventory_hash,
                                )
                                .map_err(|error| error.to_string())?;
                                inventory_hash
                            };
                            writer_connection
                                .execute_batch("PRAGMA wal_checkpoint(FULL);")
                                .map_err(|error| error.to_string())?;
                            std::fs::File::open(&migration_database_path)
                                .and_then(|file| file.sync_all())
                                .map_err(|error| error.to_string())?;
                            let sqlite_pointer = LocalStoreAuthorityPointerV1::Sqlite {
                                generation_id: migration_generation_id.clone(),
                                store_id: migration_store_id.clone(),
                                activated_migration_id: Some(migration_id.clone()),
                            };
                            let cas_result = cas_authority(
                                &migration_layout,
                                Some(&legacy_pointer),
                                &sqlite_pointer,
                                fault.take_authority_cutover_fault(),
                            );
                            // A successful rename acknowledgement is not the
                            // cutover proof either: always fresh-read the
                            // checksummed pointer after the CAS attempt.
                            let fresh = read_authority(&migration_layout)
                                .map_err(|error| error.to_string())?;
                            if fresh != Some(sqlite_pointer.clone()) {
                                return Err(if cas_result.is_err() {
                                    "migration authority cutover was not confirmed".to_string()
                                } else {
                                    "migration authority changed after confirmed cutover"
                                        .to_string()
                                });
                            }
                            // A fresh Sqlite pointer is not sufficient proof:
                            // bind it back to the verified staging parity row.
                            verify_activated_migration(&writer_connection, &migration_id)
                                .map_err(|error| error.to_string())?;
                            Ok(())
                        })();
                        match migration_result {
                            Ok(()) => migration_admission_worker.store(3, Ordering::Release),
                            Err(error) => {
                                let correlation_id = correlation_id();
                                log::error!(
                                    "local event store migration blocked [{correlation_id}]: {error}"
                                );
                                let activation_is_sealed =
                                    activated_migration_inventory_hash(
                                        &writer_connection,
                                        &migration_id,
                                    )
                                    .ok()
                                    .flatten()
                                    .is_some();
                                if !activation_is_sealed {
                                    // Preserve the last bounded ordinal/count
                                    // checkpoint. A safe failure is an additive
                                    // terminal projection, never progress reset.
                                    let saved_checkpoint: Option<String> = writer_connection
                                        .query_row(
                                            "SELECT checkpoint FROM local_store_migrations WHERE migration_id = ?1",
                                            rusqlite::params![migration_id],
                                            |row| row.get(0),
                                        )
                                        .ok();
                                    let mut checkpoint = saved_checkpoint
                                        .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
                                        .and_then(|value| value.as_object().cloned())
                                        .unwrap_or_default();
                                    checkpoint.insert(
                                        "safe_failure".to_string(),
                                        serde_json::Value::String("migration_blocked".to_string()),
                                    );
                                    checkpoint.insert(
                                        "correlation_id".to_string(),
                                        serde_json::Value::String(correlation_id),
                                    );
                                    checkpoint.insert(
                                        "read_only".to_string(),
                                        serde_json::Value::Bool(true),
                                    );
                                    let checkpoint =
                                        serde_json::Value::Object(checkpoint).to_string();
                                    let _ = writer_connection.execute(
                                        "UPDATE local_store_migrations SET phase = 'failed', checkpoint = ?2, revision = revision + 1 WHERE migration_id = ?1",
                                        rusqlite::params![migration_id, checkpoint],
                                    );
                                }
                                migration_admission_worker.store(2, Ordering::Release);
                            }
                        }
                    }
                    loop {
                        let request = match queue.pop_with_timeout(std::time::Duration::from_millis(50)) {
                            QueuePop::Request(request) => request,
                            QueuePop::Idle => {
                                if let Err(error) = cleanup_compacted_shutdown_details(&writer_connection) {
                                    log::warn!("bounded shutdown detail cleanup failed: {error}");
                                }
                                continue;
                            }
                            QueuePop::Closed => break,
                        };
                        if fault.worker_stopped() {
                            // Crash-equivalent: drop this and all queued
                            // requests; callers observe reply loss.
                            queue.close();
                            drop(request);
                            break;
                        }
                        let now_ms = clock.now_ms().max(0);
                        let result =
                            execute_commit(&writer_connection, &request.prepared, now_ms, &fault);
                        if let Ok(CommitBatchResult::Committed(batch)) = &result {
                            if let Some((_, last)) = batch.sequence_range {
                                let _ = notifications.send(Arc::new(BroadcastSignal {
                                    commit_id: batch.commit_id.as_str().to_string(),
                                    max_global_sequence: last.value(),
                                }));
                            }
                        }
                        if fault.take_drop_reply() {
                            drop(request.reply);
                            continue;
                        }
                        let _ = request.reply.send(result);
                        if let Err(error) = cleanup_compacted_shutdown_details(&writer_connection) {
                            log::warn!("bounded shutdown detail cleanup failed: {error}");
                        }
                    }
                })
                .map_err(LocalEventStoreOpenError::Io)?;
        }

        // Bounded reader pool: dedicated threads with read-only connections.
        for index in 0..READER_POOL_SIZE {
            let readers = Arc::clone(&readers);
            let connection = open_reader(&database_path)?;
            std::thread::Builder::new()
                .name(format!("local-event-store-reader-{index}"))
                .spawn(move || readers.run_worker(connection))
                .map_err(LocalEventStoreOpenError::Io)?;
        }

        Ok(Arc::new(Self {
            registry: Arc::clone(&config.registry),
            #[cfg(test)]
            fault: config.fault,
            queue,
            readers,
            recovery_snapshots,
            query_context,
            notifications,
            generation_id,
            operation_binding_key,
            migration_admission,
            active_migration_id,
            _writer_lock: writer_lock,
        }))
    }

    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }

    pub fn boot_id(&self) -> &str {
        &self.query_context.boot_id
    }

    /// True only after a new installation or a verified Legacy→SQLite
    /// cutover.  While false, callers may use the legacy reader for
    /// read-only presentation, but every normal mutation is rejected by
    /// `commit_batch`.
    pub fn normal_admission_ready(&self) -> bool {
        self.migration_admission.load(Ordering::Acquire) == 1
    }

    pub fn migration_blocked(&self) -> bool {
        self.migration_admission.load(Ordering::Acquire) == 2
    }

    /// The pointer is SQLite and verified, but application read authorities
    /// must install before normal mutation admission opens.
    pub fn cutover_ready(&self) -> bool {
        self.migration_admission.load(Ordering::Acquire) == 3
    }

    pub fn open_normal_admission_after_authority_install(&self) -> bool {
        self.migration_admission
            .compare_exchange(3, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn active_migration_id(&self) -> Option<&str> {
        self.active_migration_id.as_deref()
    }

    fn is_migration_safe_quit_batch(&self, batch: &LocalAtomicBatch) -> bool {
        if !matches!(
            batch.idempotency.operation_kind,
            crate::domain::local_event::CommitOperationKind::ApplicationQuit
        ) || !batch.events.is_empty()
            || !batch.expected_heads.is_empty()
        {
            return false;
        }
        if batch.state_mutations.len() == 2 {
            let binding = batch
                .state_mutations
                .iter()
                .find_map(|mutation| match mutation {
                    LocalStateMutation::OperationBinding(binding) => Some(binding),
                    _ => None,
                });
            let operation = batch
                .state_mutations
                .iter()
                .find_map(|mutation| match mutation {
                    LocalStateMutation::OperationRecord(operation) => Some(operation),
                    _ => None,
                });
            let flight = batch
                .state_mutations
                .iter()
                .find_map(|mutation| match mutation {
                    LocalStateMutation::MigrationQuitFlight(flight) => Some(flight),
                    _ => None,
                });
            let join = matches!((binding, flight), (Some(binding), Some(flight))
                if binding.key.kind == crate::domain::local_event::OperationKind::ApplicationQuit
                    && binding.operation_id == flight.operation_id
                    && self.active_migration_id() == Some(flight.migration_id.as_str()));
            let settlement = matches!((operation, flight), (Some(operation), Some(flight))
                if operation.kind == crate::domain::local_event::OperationKind::ApplicationQuit
                    && operation.operation_id == flight.operation_id
                    && self.active_migration_id() == Some(flight.migration_id.as_str())
                    && migration_quit_operation_matches(operation, &flight.migration_id));
            return join || settlement;
        }
        if batch.state_mutations.len() != 3 {
            return false;
        }
        let binding = batch
            .state_mutations
            .iter()
            .find_map(|mutation| match mutation {
                LocalStateMutation::OperationBinding(binding) => Some(binding),
                _ => None,
            });
        let operation = batch
            .state_mutations
            .iter()
            .find_map(|mutation| match mutation {
                LocalStateMutation::OperationRecord(operation) => Some(operation),
                _ => None,
            });
        let flight = batch
            .state_mutations
            .iter()
            .find_map(|mutation| match mutation {
                LocalStateMutation::MigrationQuitFlight(flight) => Some(flight),
                _ => None,
            });
        let (Some(binding), Some(operation), Some(flight)) = (binding, operation, flight) else {
            return false;
        };
        if binding.key.kind != crate::domain::local_event::OperationKind::ApplicationQuit
            || operation.kind != crate::domain::local_event::OperationKind::ApplicationQuit
            || binding.operation_id != operation.operation_id
            || operation.operation_id != flight.operation_id
            || self.active_migration_id() != Some(flight.migration_id.as_str())
        {
            return false;
        }
        migration_quit_operation_matches(operation, &flight.migration_id)
    }

    #[cfg(test)]
    pub fn fault_injector(&self) -> &Arc<FaultInjector> {
        &self.fault
    }

    #[cfg(test)]
    pub(crate) fn reader_pool_for_test(&self) -> Arc<ReaderPool> {
        Arc::clone(&self.readers)
    }

    /// Validate and encode a batch before queue admission (design step 1).
    fn prepare(&self, batch: LocalAtomicBatch) -> Result<PreparedBatch, CommitBatchError> {
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
            decoded_bytes,
            critical,
        })
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
}

impl crate::usecase::agent_session::operation::RecoveryResultCanonicalizer for LocalEventStore {
    fn canonicalize_recovery_result(
        &self,
        outcome: crate::domain::local_event::RecoveryResultOutcomeRecord,
        classification: crate::domain::agent_session::events::RecoveryResultClassification,
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

impl crate::usecase::agent_session::operation::OperationBindingAuthority for LocalEventStore {
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
        let migration_safe_quit = self.is_migration_safe_quit_batch(&batch);
        match self.migration_admission.load(Ordering::Acquire) {
            0 if !migration_safe_quit => {
                return Err(CommitBatchError::StorageUnavailable {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::PersistFailure,
                        true,
                        "Local data migration is in progress.",
                        correlation_id(),
                    ),
                })
            }
            2 if !migration_safe_quit => {
                return Err(CommitBatchError::StorageUnavailable {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::MigrationBlocked,
                        false,
                        "Local data migration requires supervision.",
                        correlation_id(),
                    ),
                })
            }
            3 if !migration_safe_quit => {
                return Err(CommitBatchError::StorageUnavailable {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::PersistFailure,
                        true,
                        "Local data authority activation is still in progress.",
                        correlation_id(),
                    ),
                })
            }
            _ => {}
        }
        let identity = batch.commit_id.clone();
        let prepared = self.prepare(batch)?;
        let (reply, receiver) = oneshot::channel();
        match self.queue.admit(WriteRequest { prepared, reply }) {
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

    fn subscribe(&self, after: GlobalSequence) -> LocalEventSubscription {
        let _ = after; // Subscribers replay from `after` through load_stream.
        let receiver = self.notifications.subscribe();
        let stream = stream::unfold(receiver, |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(signal) => {
                        let Ok(commit_id) = CommitIdentity::parse(&signal.commit_id) else {
                            continue;
                        };
                        let Ok(max_global_sequence) =
                            GlobalSequence::new(signal.max_global_sequence)
                        else {
                            continue;
                        };
                        return Some((
                            LocalEventSignal::Committed {
                                commit_id,
                                max_global_sequence,
                            },
                            receiver,
                        ));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        return Some((LocalEventSignal::ReplayRequired, receiver));
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        });
        LocalEventSubscription::new(Box::pin(stream))
    }
}

impl Drop for LocalEventStore {
    fn drop(&mut self) {
        self.queue.close();
        self.readers.close();
        self.recovery_snapshots.close();
    }
}
