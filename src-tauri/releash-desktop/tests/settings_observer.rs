#[tokio::test]
async fn test_設定観測_watchの通知で更新を渡し再接続した購読へ切り替える() {
    use releash_desktop::test_support::integration::settings_observer::observe;
    // Given
    let (sources, receiver) = tokio::sync::watch::channel(None);
    let (delivered, mut updates) = tokio::sync::mpsc::unbounded_channel();
    let observer = tokio::spawn(observe(receiver, move |value| {
        delivered.send(value).unwrap();
    }));
    let (first, receiver) = tokio::sync::watch::channel(Some(1));
    // When / Then
    sources.send_replace(Some(receiver));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap(),
        Some(1)
    );
    first.send_replace(Some(2));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap(),
        Some(2)
    );
    let (second, receiver) = tokio::sync::watch::channel(Some(3));
    sources.send_replace(Some(receiver));
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), updates.recv())
            .await
            .unwrap(),
        Some(3)
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
