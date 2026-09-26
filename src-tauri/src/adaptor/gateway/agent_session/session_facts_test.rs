use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition;
use crate::adaptor::presenter::connect::ConnectFailure;
use connectrpc::ErrorCode;

#[tokio::test]
async fn test_session読取_親と自身の実行定義を解釈せず接続情報を取得できる() {
    // Given
    for unavailable in ["main", "session", "unused"] {
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
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
            crate::domain::provider_lifecycle::ProviderKind::Codex
        );
        assert_eq!(context.worktree_path, "/repo");
        assert!(records
            .iter()
            .all(|record| record.meta.node_execution_id == "tree-session"));
        assert!(records
            .iter()
            .all(|record| !matches!(record.fact, crate::domain::workflow::NodeFact::Started(_))));
        assert_eq!(records.len(), 1);
    }
}

#[tokio::test]
async fn test_session読取_root欠落と対象provider欠落は接続情報取得エラーになる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
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
async fn test_session読取_sql障害とroot欠損を区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
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
        if failure.nature == crate::domain::failure::TechnicalFailureNature::Other
        && matches!(failure.source, crate::domain::failure::StorageFailureSource::Query(LocalEventQueryError::Internal { .. })))
    );
    let query_error =
        AgentSessionQueryError::from(read_session_context(&backend, &location).await.unwrap_err());
    assert_eq!(query_error.connect_code(), ErrorCode::Internal);
    assert!(matches!(query_error, AgentSessionQueryError::Store(failure)
        if failure.nature == crate::domain::failure::TechnicalFailureNature::Other
        && matches!(failure.source, crate::domain::failure::StorageFailureSource::Query(LocalEventQueryError::Internal { .. }))));
}

#[test]
fn test_session読取_混雑と期限切れをデータ破損扱いしない() {
    // Given
    for (error, expected) in [
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
    ] {
        // When / Then
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::Read(error.clone()))
                .connect_code(),
            expected
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::Read(error)).connect_code(),
            expected
        );
    }
    let corrupt = LocalEventQueryError::Corrupt {
        correlation_id: "corrupt".into(),
    };
    assert_eq!(
        AgentSessionRepositoryError::from(SessionContextReadError::Read(corrupt.clone())),
        AgentSessionRepositoryError::Corrupt
    );
    assert_eq!(
        AgentSessionQueryError::from(SessionContextReadError::Read(corrupt)),
        AgentSessionQueryError::Corrupt
    );
}

#[test]
fn test_session読取_実効cwdの一時障害と破損をrepositoryとqueryへ区別して返す() {
    use crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError;
    // Given
    for (error, expected) in [
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (
            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                &rusqlite::Error::InvalidQuery,
            ),
            ErrorCode::Internal,
        ),
    ] {
        // When / Then
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Read(error.clone())
            ))
            .connect_code(),
            expected
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Read(error)
            ))
            .connect_code(),
            expected
        );
    }
    for reason in [
        "parent execution is missing",
        "invalid worktree execution ancestry",
        "invalid worktree definition",
    ] {
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Corrupt(reason.into())
            )),
            AgentSessionRepositoryError::Corrupt
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Corrupt(reason.into())
            )),
            AgentSessionQueryError::Corrupt
        );
    }
}

#[tokio::test]
async fn test_session読取_子sessionにもrootのarchiveとrestoreを反映する() {
    use crate::domain::workflow::services::fact_replay::derive_session_facts;
    use crate::domain::workflow::{NodeFact, NodeFactMeta};
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
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
        NodeFact::ArchiveRequested(crate::domain::workflow::ArchiveRequestedFact {
            reason: "manual".into(),
            archived_at: 0.0,
        }),
        NodeFact::RestoreRequested,
    ] {
        fact_log::append_single_fact(&store, &root, &fact, 100)
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
