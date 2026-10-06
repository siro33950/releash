pub(crate) mod tests {
    use crate::domain::local_event::CommitIdentity;
    use releash_lib::test_support::integration::domain::local_event::LocalEventTransactionRepository;

    use super::super::*;
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::domain::local_event::{
        CommitOperationKind, IdempotencyBinding, LocalEventQueryResult,
    };
    use crate::infrastructure::local_event_store_connection::open_writer;

    #[test]
    pub fn unrelated_files_are_never_a_read_fallback_without_sqlite_authority() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        std::fs::create_dir_all(root.path().join("sessions/session-legacy"))
            .expect("legacy session fixture");

        let error = match LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ) {
            Ok(_) => panic!("unrelated files must not become a cross-process read authority"),
            Err(error) => error,
        };

        assert_eq!(error, STORE_NOT_READY);
    }

    #[test]
    pub fn sqlite_authority_requires_current_schema() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        drop(writer);

        let database_path = StoreLayout::new(root.path()).database_path();
        let connection = open_writer(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("maintenance connection");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("stale schema fixture");
        drop(connection);

        let error = match LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ) {
            Ok(_) => panic!("stale schema must not publish canonical session state"),
            Err(error) => error,
        };
        assert_eq!(error, STORE_NOT_READY);
    }

    #[test]
    pub fn reader_snapshot_validation_requires_only_markers_and_installation_identity() {
        let root = tempfile::TempDir::new().expect("lightweight reader validation");
        let database_path = root.path().join("marker-only.sqlite3");
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .pragma_update(
                None,
                "application_id",
                crate::adaptor::gateway::local_event_store::schema::APPLICATION_ID,
            )
            .unwrap();
        connection
            .pragma_update(
                None,
                "user_version",
                crate::adaptor::gateway::local_event_store::schema::CURRENT_SCHEMA_VERSION,
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
    pub async fn writer_commit_and_wal_checkpoint_preserve_database_file_identity() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        let reader = LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("concurrent canonical reader");
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
        let maintenance = open_writer(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("checkpoint connection");
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
    pub async fn reader_allows_bounded_queries_but_fails_mutation_and_resolution_closed() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        let reader = LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("concurrent canonical reader");

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
            CommitBatchError::StorageAccessRequired { failure }
                if failure.kind == SessionOperationFailureKind::PersistFailure
                    && failure.nature == crate::domain::failure::TechnicalFailureNature::Other
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
            LocalEventQueryError::CanonicalWriterRequired
        ));
    }

    #[tokio::test]
    pub async fn reader_fails_closed_when_schema_changes_after_open() {
        // Given
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let maintenance = open_writer(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("maintenance connection");
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
    pub async fn reader_fails_closed_when_installation_changes_after_open() {
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let maintenance = open_writer(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("maintenance connection");
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
    pub async fn reader_fails_closed_when_database_file_is_replaced_after_open() {
        // Given
        let root = tempfile::TempDir::new().expect("read-only app data");
        let writer = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("canonical writer");
        drop(writer);
        let reader = LocalEventReadStore::open(
            root.path(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .expect("canonical reader");
        let database_path = StoreLayout::new(root.path()).database_path();
        let replaced_path = root.path().join("replaced-local-event-store.sqlite3");
        std::fs::rename(&database_path, &replaced_path).expect("retain replaced fixture");
        let replacement = LocalEventStore::open(LocalEventStoreConfig::production(
            root.path().to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
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
