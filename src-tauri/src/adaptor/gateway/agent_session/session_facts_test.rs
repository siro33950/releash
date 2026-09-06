use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition;

#[test]
fn test_session読取_親と自身の実行定義を解釈せず接続情報を取得できる() {
    // Given
    for unavailable in ["main", "session", "unused"] {
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        seed_unavailable_definition(&store, "tree", "/repo", unavailable);
        let backend = FactLogReadBackend::Live(store);
        let location = locate_session(&backend, "tree-session").unwrap().unwrap();

        // When
        let context = read_session_context(&backend, &location).unwrap();
        let records = read_session_records(&backend, &location).unwrap();

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

#[test]
fn test_session読取_root欠落と対象provider欠落は接続情報取得エラーになる() {
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
        .unwrap_err()
        .to_string()
        .contains("root is missing"));
    seed_unavailable_definition(&store, "tree", "/repo", "unused");
    let missing = SessionLocation {
        node_name: "missing".into(),
        ..location
    };
    assert!(read_session_context(&backend, &missing)
        .unwrap_err()
        .to_string()
        .contains("provider is unavailable"));
}

#[test]
fn test_session読取_sql障害とroot欠損を区別する() {
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
    let error = read_session_context(&backend, &location).unwrap_err();
    assert_eq!(
        AgentSessionRepositoryError::from(error),
        AgentSessionRepositoryError::Corrupt
    );
    rusqlite::Connection::open(directory.path().join("local-event-store.sqlite3"))
        .unwrap()
        .execute("DROP TABLE node_events", [])
        .unwrap();
    let error = read_session_context(&backend, &location).unwrap_err();
    assert!(matches!(
        &error,
        SessionContextReadError::Read(LocalEventQueryError::StorageUnavailable { .. })
    ));
    assert_eq!(
        AgentSessionRepositoryError::from(error),
        AgentSessionRepositoryError::Unavailable
    );
    assert_eq!(
        AgentSessionQueryError::from(read_session_context(&backend, &location).unwrap_err()),
        AgentSessionQueryError::Unavailable
    );
}

#[test]
fn test_session読取_混雑と期限切れをデータ破損扱いしない() {
    // Given
    for error in [
        LocalEventQueryError::QueryBusy,
        LocalEventQueryError::DeadlineExceeded,
    ] {
        // When / Then
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::Read(error.clone())),
            AgentSessionRepositoryError::Unavailable
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::Read(error)),
            AgentSessionQueryError::Unavailable
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
