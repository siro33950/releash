//! Read-only local-event repository for CLI processes that may run while the
//! desktop process owns the single SQLite writer lock.
//!
//! Opening this adapter never creates or migrates an authority. Its mutation
//! entry point always fails closed; reads validate the durable authority
//! pointer before and after each SQLite snapshot.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::stream;
use rusqlite::OptionalExtension;

use crate::adaptor::gateway::local_event_store::authority::{
    read_authority, LocalStoreAuthorityPointerV1, StoreLayout,
};
use crate::adaptor::gateway::local_event_store::clock::SystemStoreClock;
use crate::adaptor::gateway::local_event_store::connection::open_reader;
use crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry;
use crate::adaptor::gateway::local_event_store::migration::verify_activated_migration_anchor;
use crate::adaptor::gateway::local_event_store::projection_record_codec::canonical_mutation_identity_v1 as canonical_projection_mutation_identity_v1;
use crate::adaptor::gateway::local_event_store::reader::{
    load_stream_page, run_query, QueryContext,
};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitResolution, DomainEventPage,
    GlobalSequence, LoadStreamRequest, LocalAtomicBatch, LocalEventQuery, LocalEventQueryError,
    LocalEventQueryResult, LocalEventSignal, LocalEventSubscription,
    LocalEventTransactionRepository, LocalStateMutation, SafeOperationFailure,
    SessionOperationFailureKind,
};

const AUTHORITY_NOT_READY: &str =
    "canonical local event authority is not ready; legacy session fallback is disabled";

pub(crate) struct LocalEventReadStore {
    layout: StoreLayout,
    authority: LocalStoreAuthorityPointerV1,
    database_path: PathBuf,
    query_context: Arc<QueryContext>,
}

impl LocalEventReadStore {
    pub(crate) fn open(app_data_root: &Path) -> Result<Arc<Self>, String> {
        let layout = StoreLayout::new(app_data_root);
        let authority = read_authority(&layout)
            .map_err(|error| format!("failed to read canonical local event authority: {error}"))?
            .ok_or_else(|| AUTHORITY_NOT_READY.to_string())?;
        let LocalStoreAuthorityPointerV1::Sqlite {
            generation_id,
            store_id,
            activated_migration_id,
        } = &authority
        else {
            return Err(AUTHORITY_NOT_READY.to_string());
        };
        let database_path = layout.generation_database_path(generation_id);
        let connection = open_reader(&database_path)
            .map_err(|error| format!("failed to open canonical local event reader: {error}"))?;
        let metadata = connection
            .query_row(
                "SELECT schema_version, store_id, generation_id, cursor_hmac_key, boot_id, health
                 FROM store_metadata WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("failed to validate canonical local event metadata: {error}"))?
            .ok_or_else(|| "canonical local event metadata is missing".to_string())?;
        if metadata.0 != 1 || metadata.1 != *store_id || metadata.2 != *generation_id {
            return Err(
                "canonical local event metadata does not match the authority pointer".to_string(),
            );
        }
        if metadata.5 != "ok" {
            return Err("canonical local event store is not ready for public reads".to_string());
        }
        if let Some(migration_id) = activated_migration_id {
            verify_activated_migration_anchor(&connection, migration_id).map_err(|error| {
                format!("canonical local event migration proof is invalid: {error}")
            })?;
        }
        if read_authority(&layout)
            .map_err(|error| {
                format!("failed to revalidate canonical local event authority: {error}")
            })?
            .as_ref()
            != Some(&authority)
        {
            return Err("canonical local event authority changed during open".to_string());
        }
        Ok(Arc::new(Self {
            layout,
            authority,
            database_path,
            query_context: Arc::new(QueryContext {
                registry: Arc::new(EventCodecRegistry::new()),
                cursor_key: metadata.3,
                boot_id: metadata.4,
                clock: Arc::new(SystemStoreClock),
            }),
        }))
    }

    pub(crate) fn generation_id(&self) -> &str {
        let LocalStoreAuthorityPointerV1::Sqlite { generation_id, .. } = &self.authority else {
            unreachable!("read-only local event store opens only SQLite authority")
        };
        generation_id
    }

    async fn read<T, F>(&self, operation: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection, &QueryContext) -> Result<T, LocalEventQueryError>
            + Send
            + 'static,
    {
        let layout = self.layout.clone();
        let authority = self.authority.clone();
        let database_path = self.database_path.clone();
        let query_context = Arc::clone(&self.query_context);
        tokio::task::spawn_blocking(move || {
            if read_authority(&layout).ok().flatten().as_ref() != Some(&authority) {
                return Err(LocalEventQueryError::Corrupt {
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }
            let connection = open_reader(&database_path).map_err(|_| {
                LocalEventQueryError::StorageUnavailable {
                    failure: SafeOperationFailure::new(
                        SessionOperationFailureKind::StorageUnavailable,
                        true,
                        "local event store read failed",
                        uuid::Uuid::new_v4().to_string(),
                    ),
                }
            })?;
            let result = operation(&connection, &query_context)?;
            if read_authority(&layout).ok().flatten().as_ref() != Some(&authority) {
                return Err(LocalEventQueryError::Corrupt {
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|_| LocalEventQueryError::Internal {
            correlation_id: uuid::Uuid::new_v4().to_string(),
        })?
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
    fn legacy_files_are_never_a_read_fallback_without_sqlite_authority() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        std::fs::create_dir_all(root.path().join("sessions/session-legacy"))
            .expect("legacy session fixture");

        let error = match LocalEventReadStore::open(root.path()) {
            Ok(_) => panic!("legacy source must not become a cross-process read authority"),
            Err(error) => error,
        };

        assert_eq!(error, AUTHORITY_NOT_READY);
    }

    #[test]
    fn sqlite_authority_requires_ready_store_health() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .expect("canonical writer");
        let generation_id = writer.generation_id().to_string();
        drop(writer);

        let database_path = StoreLayout::new(root.path()).generation_database_path(&generation_id);
        let connection = open_writer(&database_path).expect("maintenance connection");
        connection
            .execute(
                "UPDATE store_metadata SET health = 'recovering' WHERE id = 1",
                [],
            )
            .expect("recovering health fixture");
        drop(connection);

        let error = match LocalEventReadStore::open(root.path()) {
            Ok(_) => panic!("recovering store must not publish canonical session state"),
            Err(error) => error,
        };
        assert!(error.contains("not ready for public reads"), "{error}");
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
                generation_id: reader.generation_id().to_string(),
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
}
