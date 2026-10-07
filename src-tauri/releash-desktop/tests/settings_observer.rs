#[tokio::test]
async fn test_設定観測_watchの通知で更新を渡し再接続した購読へ切り替える() {
    use releash_desktop::test_support::integration::settings_observer::observe;
    // Given
    let (sources, receiver) = tokio::sync::watch::channel(None);
    let (delivered, mut updates) = tokio::sync::mpsc::unbounded_channel();
    let observer = tokio::spawn(observe(
        receiver,
        |source: &tokio::sync::watch::Receiver<Option<i32>>| source.clone(),
        move |_, value| {
            let delivered = delivered.clone();
            async move {
                delivered.send(value).unwrap();
            }
        },
    ));
    let (first, receiver) = tokio::sync::watch::channel(Some(1));
    // When / Then
    sources.send_replace(Some(std::sync::Arc::new(receiver)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), updates.recv())
            .await
            .is_err()
    );
    first.send_replace(Some(2));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap(),
        Some(2)
    );
    let (second, receiver) = tokio::sync::watch::channel(Some(3));
    sources.send_replace(Some(std::sync::Arc::new(receiver)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), updates.recv())
            .await
            .is_err()
    );
    first.send_replace(Some(4));
    second.send_replace(Some(5));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap(),
        Some(5)
    );
    sources.send_replace(None);
    drop(second);
    drop(first);
    drop(sources);
    tokio::time::timeout(std::time::Duration::from_secs(1), observer)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn test_設定観測_初回受信後でobserver開始前に届いた更新を引き継ぐ() {
    use releash_desktop::test_support::integration::settings_observer::observe;
    // Given
    let (settings, mut receiver) = tokio::sync::watch::channel(None);
    settings.send_replace(Some(1));
    assert_eq!(
        *receiver.wait_for(|value| value.is_some()).await.unwrap(),
        Some(1)
    );
    let (sources, observed_sources) =
        tokio::sync::watch::channel(Some(std::sync::Arc::new(receiver)));
    settings.send_replace(Some(2));
    let (delivered, mut values) = tokio::sync::mpsc::unbounded_channel();
    // When
    let observer = tokio::spawn(observe(
        observed_sources,
        |source: &tokio::sync::watch::Receiver<Option<i32>>| source.clone(),
        move |_, value| {
            let delivered = delivered.clone();
            async move {
                delivered.send(value).unwrap();
            }
        },
    ));
    // Then
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), values.recv())
            .await
            .unwrap(),
        Some(2)
    );
    assert!(values.try_recv().is_err());
    drop(sources);
    observer.await.unwrap();
}
