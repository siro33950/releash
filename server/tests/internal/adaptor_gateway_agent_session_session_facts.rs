use connectrpc::ErrorCode;
use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::platform::LocalEventQueryError;
use releashd::test_support::integration::sessions::locate_session;
use releashd::test_support::integration::sessions::read_session_context;
use releashd::test_support::integration::sessions::read_session_records;
use releashd::test_support::integration::sessions::AgentSessionQueryError;
use releashd::test_support::integration::sessions::AgentSessionRepositoryError;
use releashd::test_support::integration::sessions::SessionContextReadError;
use releashd::test_support::integration::sessions::SessionLocation;
use releashd::test_support::integration::transport::ConnectFailure;
use releashd::test_support::integration::workflow::seed_unavailable_definition;
use releashd::test_support::integration::workflow::FactLogReadBackend;
use releashd::test_support::integration::workflow::NodeFactMeta;
use releashd::test_support::integration::workflow::NodeKindName;

#[tokio::test]
pub async fn test_session読取_親と自身の実行定義を解釈せず接続情報を取得できる() {
    // Given
    for unavailable in ["main", "session", "unused"] {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        seed_unavailable_definition(&store, "tree", "/repo", unavailable).await;
        let backend = FactLogReadBackend::Live(store);
        let location = locate_session(&backend, "tree-session")
            .await
            .unwrap()
            .unwrap();

        // When
        let context = read_session_context(&backend, &location).await.unwrap();
        let records = read_session_records(&backend, &location).await.unwrap();

        // Then
        assert_eq!(
            context.provider,
            releashd::test_support::integration::providers::ProviderKind::Codex
        );
        assert_eq!(context.worktree_path, "/repo");
        assert!(records
            .iter()
            .all(|record| record.meta.node_execution_id == "tree-session"));
        assert!(records.iter().all(|record| !matches!(
            record.fact,
            releashd::test_support::integration::workflow::NodeFact::Started(_)
        )));
        assert_eq!(records.len(), 1);
    }
}

#[tokio::test]
pub async fn test_session読取_root欠落と対象provider欠落は接続情報取得エラーになる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let backend = FactLogReadBackend::Live(store.clone());
    let location = SessionLocation {
        tree_id: "tree".into(),
        node_execution_id: "tree-session".into(),
        parent_id: Some("tree".into()),
        node_name: "session".into(),
        attempt: 1,
    };

    // When / Then
    assert!(read_session_context(&backend, &location)
        .await
        .unwrap_err()
        .to_string()
        .contains("root is missing"));
    seed_unavailable_definition(&store, "tree", "/repo", "unused").await;
    let missing = SessionLocation {
        node_name: "missing".into(),
        ..location
    };
    assert!(read_session_context(&backend, &missing)
        .await
        .unwrap_err()
        .to_string()
        .contains("provider is unavailable"));
}

#[tokio::test]
pub async fn test_session読取_sql障害とroot欠損を区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let backend = FactLogReadBackend::Live(store);
    let location = SessionLocation {
        tree_id: "missing".into(),
        node_execution_id: "missing".into(),
        parent_id: None,
        node_name: "session".into(),
        attempt: 1,
    };

    // When / Then
    let error = read_session_context(&backend, &location).await.unwrap_err();
    assert_eq!(
        AgentSessionRepositoryError::from(error),
        AgentSessionRepositoryError::Corrupt
    );
    rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute("DROP TABLE node_events", [])
        .unwrap();
    let error = read_session_context(&backend, &location).await.unwrap_err();
    assert!(matches!(
        &error,
        SessionContextReadError::Read(LocalEventQueryError::Internal { .. })
    ));
    let repository_error = AgentSessionRepositoryError::from(error);
    assert_eq!(repository_error.connect_code(), ErrorCode::Internal);
    assert!(
        matches!(repository_error, AgentSessionRepositoryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Other
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Query(LocalEventQueryError::Internal { .. })))
    );
    let query_error =
        AgentSessionQueryError::from(read_session_context(&backend, &location).await.unwrap_err());
    assert_eq!(query_error.connect_code(), ErrorCode::Internal);
    assert!(matches!(query_error, AgentSessionQueryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Other
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Query(LocalEventQueryError::Internal { .. }))));
}

#[tokio::test]
pub async fn test_session読取_子sessionにもrootのarchiveとrestoreを反映する() {
    use releashd::test_support::integration::workflow::derive_session_facts;
    use releashd::test_support::integration::workflow::NodeFact;

    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    seed_unavailable_definition(&store, "tree", "/repo", "unused").await;
    let backend = FactLogReadBackend::Live(store.clone());
    let location = locate_session(&backend, "tree-session")
        .await
        .unwrap()
        .unwrap();
    let root = NodeFactMeta {
        tree_id: "tree".into(),
        node_execution_id: "tree".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: NodeKindName::Sequence,
        attempt: 1,
    };
    for fact in [
        NodeFact::ArchiveRequested(
            releashd::test_support::integration::workflow::ArchiveRequestedFact {
                reason: "manual".into(),
                archived_at: 0.0,
            },
        ),
        NodeFact::RestoreRequested,
    ] {
        releashd::test_support::integration::workflow::append_single_fact(
            &store, &root, &fact, 100,
        )
        .await
        .unwrap();
        let records = read_session_records(&backend, &location).await.unwrap();
        let view = derive_session_facts(&records, &location.node_execution_id, "tree-session");
        assert_eq!(view.archived, matches!(fact, NodeFact::ArchiveRequested(_)));
        if matches!(fact, NodeFact::RestoreRequested) {
            assert!(view.exited);
        }
    }
}
