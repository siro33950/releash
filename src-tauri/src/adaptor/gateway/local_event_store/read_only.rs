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
    load_stream_page, run_query, QueryContext,
};
use crate::adaptor::gateway::local_event_store::schema::validate_current_schema;
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
    installation_id: String,
    query_context: Arc<QueryContext>,
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
        validate_current_schema(&connection).map_err(|_| STORE_NOT_READY.to_string())?;
        let metadata: (String, Vec<u8>, String) = connection
            .query_row(
                "SELECT installation_id, cursor_hmac_key, process_instance_id
                 FROM store_metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| STORE_NOT_READY.to_string())?;
        Ok(Arc::new(Self {
            database_path,
            installation_id: metadata.0,
            query_context: Arc::new(QueryContext {
                registry: Arc::new(EventCodecRegistry::new()),
                cursor_key: metadata.1,
                process_instance_id: metadata.2,
                clock: Arc::new(SystemStoreClock),
            }),
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
        let database_path = self.database_path.clone();
        let installation_id = self.installation_id.clone();
        let query_context = Arc::clone(&self.query_context);
        tokio::task::spawn_blocking(move || {
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
            validate_current_schema(&connection).map_err(|_| LocalEventQueryError::Corrupt {
                correlation_id: uuid::Uuid::new_v4().to_string(),
            })?;
            let current_installation_id: String = connection
                .query_row(
                    "SELECT installation_id FROM store_metadata WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| LocalEventQueryError::Corrupt {
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                })?;
            if current_installation_id != installation_id {
                return Err(LocalEventQueryError::Corrupt {
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                });
            }
            let result = operation(&connection, &query_context)?;
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
}
