pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use super::super::*;
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
        LocalEventStore::open(LocalEventStoreConfig::production(
            root.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .expect("file-backed local event store")
    }

    fn open_store_with_fault(
        root: &Path,
        fault: Arc<FaultInjector>,
        observer: Arc<dyn AppDataPathObserver>,
    ) -> Arc<LocalEventStore> {
        let mut config = LocalEventStoreConfig::production(
            root.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        );
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
        let connection = open_existing_writer(
            &path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
                 INSERT INTO session_projection (
                     session_id, projection, revision, commit_id
                 ) VALUES (
                     'maintenance-session', '{}', 1, 'maintenance-preserved'
                 );
                 INSERT INTO node_events (
                     tree_id, seq, node_execution_id, parent_id, node_name, kind,
                     attempt, event_type, detail, timestamp
                 ) VALUES (
                     'maintenance-execution', 1, 'maintenance-node-execution', NULL,
                     'main', 'session', 1, 'started', '{}', 1
                 );
                 UPDATE store_metadata
                    SET next_global_sequence = 2
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
        const TABLES: [&str; 5] = [
            "logical_commits",
            "stream_heads",
            "events",
            "session_projection",
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
                        next_global_sequence, health
                 FROM store_metadata WHERE id = 1",
                [],
                |row| {
                    (0..6)
                        .map(|index| row.get(index))
                        .collect::<Result<Vec<rusqlite::types::Value>, _>>()
                },
            )
            .unwrap();
        StoreSnapshot { tables, metadata }
    }

    fn snapshot_store_path(root: &Path) -> StoreSnapshot {
        let connection = open_existing_writer(
            &database_path(root),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
        snapshot_store(&connection)
    }

    fn assert_preserved_content(root: &Path, installation_id: &str) {
        let connection = open_existing_writer(
            &database_path(root),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
        crate::adaptor::gateway::local_event_store::schema::validate_current_schema(&connection)
            .unwrap();
    }

    #[test]
    pub fn test_起動時保守_非発火storeではvacuumと差し替えへ進まない() {
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
    pub async fn test_起動時保守_発火storeを縮小してデータ保持と再writeを可能にする() {
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
        let connection = open_existing_writer(
            &database_path(root.path()),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
    pub fn test_起動時保守_replace成功前の各faultで元storeを維持して一時artifactを除去する() {
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
    pub async fn test_起動時保守_replace直後のfault境界から新canonicalを次回openする() {
        let root = tempfile::TempDir::new().unwrap();
        let (size_before, installation_id) = create_fragmented_store(root.path());
        let expected_snapshot = snapshot_store_path(root.path());
        let layout = StoreLayout::new(root.path());
        let fault = Arc::new(FaultInjector::new());
        fault.arm_maintenance_fault(MaintenanceFaultPoint::AfterReplace);
        let connection = open_existing_writer(
            &database_path(root.path()),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
        prepare_vacuum_database(
            &layout,
            &connection,
            fault.as_ref(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
    pub fn test_起動時保守_vacuum書込み前からowner_only権限を持つ() {
        let root = tempfile::TempDir::new().unwrap();
        create_fragmented_store(root.path());
        let layout = StoreLayout::new(root.path());
        let connection = open_existing_writer(
            &database_path(root.path()),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
        let fault = FaultInjector::new();
        fault.arm_maintenance_fault(MaintenanceFaultPoint::BeforeVacuumInto);

        assert!(matches!(
            prepare_vacuum_database(
                &layout,
                &connection,
                &fault,
                std::sync::Arc::new(crate::common::retry::RetryLimiter::new())
            ),
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
    pub fn test_起動時保守_非utf8一時path失敗を区別して元storeを継続する() {
        use std::os::unix::ffi::OsStringExt;

        let root = tempfile::TempDir::new().unwrap();
        let (_, installation_id) = create_fragmented_store(root.path());
        let connection = open_existing_writer(
            &database_path(root.path()),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
        let invalid_root =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()));
        let layout = StoreLayout::new(&invalid_root);

        let error = prepare_vacuum_database(
            &layout,
            &connection,
            &FaultInjector::new(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
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
    pub fn test_起動時保守_非発火時は前回のstale一時artifactだけを除去する() {
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
    pub fn test_起動時保守_発火時はstale一時artifactを除去して回収を完了する() {
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
    pub fn test_起動時保守_checkpoint_busyではwalを保持して保守をskipする() {
        let root = tempfile::TempDir::new().unwrap();
        let store = open_store(root.path());
        let installation_id = store.installation_id().to_string();
        drop(store);
        let layout = StoreLayout::new(root.path());
        let database_path = layout.database_path();
        let reader = open_reader(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
        reader.execute_batch("BEGIN;").unwrap();
        reader
            .query_row("SELECT COUNT(*) FROM store_metadata", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let writer = open_existing_writer(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
        let verification = open_reader(
            &database_path,
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        )
        .unwrap();
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
