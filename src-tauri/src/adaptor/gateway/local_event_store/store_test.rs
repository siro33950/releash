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
