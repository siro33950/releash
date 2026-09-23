use super::{LocalEventStore, LocalEventStoreConfig, LocalEventStoreOpenError};

#[test]
fn test_local_event_store_複製descriptorが残っても終了後にwriter_lockを解放する() {
    // Given: 子プロセスへの継承と同様に writer lock の descriptor が複製されている
    let directory = tempfile::tempdir().unwrap();
    let config = || LocalEventStoreConfig::production(directory.path().to_path_buf());
    let store = LocalEventStore::open(config()).unwrap();
    let inherited_lock = store.writer_lock.try_clone().unwrap();
    assert!(matches!(
        LocalEventStore::open(config()),
        Err(LocalEventStoreOpenError::WriterLockHeld)
    ));

    // When: 全 worker を終了して store を閉じる
    drop(store);

    // Then: 複製 descriptor の close を待たずに同じ store を開き直せる
    let reopened = LocalEventStore::open(config()).unwrap();
    drop(inherited_lock);
    assert!(matches!(
        LocalEventStore::open(config()),
        Err(LocalEventStoreOpenError::WriterLockHeld)
    ));
    drop(reopened);
}

#[test]
fn test_node事実追記_読取後の外部追記と競合したbatchは一行も保存しない() {
    use crate::adaptor::gateway::local_event_store::node_events::{read_tree, NewNodeEventRow};
    use crate::adaptor::gateway::local_event_store::writer::NodeEventWriteError;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let row = NewNodeEventRow {
        tree_id: "tree".into(),
        node_execution_id: "node".into(),
        parent_id: None,
        node_name: "main".into(),
        kind: "session".into(),
        attempt: 1,
        event_type: "started".into(),
        session_id: None,
        detail: "{}".into(),
    };
    let head = store
        .append_node_event_blocking(row.clone(), Some(1))
        .unwrap();
    store
        .append_node_event_blocking(row.clone(), Some(2))
        .unwrap();
    // When
    let result = store.append_node_events_at_head_blocking(
        vec![(row.clone(), Some(3)), (row.clone(), Some(3))],
        Some(("tree".into(), head)),
    );
    // Then
    assert!(matches!(result, Err(NodeEventWriteError::Conflict)));
    let rows = store
        .submit_indexed_query_blocking(|connection| {
            read_tree(connection, "tree")
                .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        store
            .append_node_events_at_head_blocking(
                vec![(row, Some(4))],
                Some(("tree".into(), rows.last().unwrap().seq)),
            )
            .unwrap(),
        vec![3]
    );
}
