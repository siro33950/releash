use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::platform::drain_and_close_store;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(start_paused = true)]
pub async fn test_受入host終了_残った参照の解放を待ってstoreを閉じる() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let retained = Arc::clone(&store);
    let shutdown = drain_and_close_store(store);
    tokio::pin!(shutdown);

    assert!(futures_util::poll!(&mut shutdown).is_pending());
    drop(retained);
    shutdown.await.unwrap();

    LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
}

#[tokio::test(start_paused = true)]
pub async fn test_受入host終了_参照が解放されなければ期限付きで失敗する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let retained = Arc::clone(&store);

    let started = tokio::time::Instant::now();
    assert_eq!(
        drain_and_close_store(store).await,
        Err("timed out waiting for local event store references during shutdown".to_string())
    );
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(Arc::strong_count(&retained), 1);
}
