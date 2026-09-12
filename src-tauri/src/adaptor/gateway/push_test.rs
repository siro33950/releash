use super::*;

#[tokio::test]
async fn test_push購読_frameと欠落と終了をgateway境界で返す() {
    // Given
    let (sender, receiver) = tokio::sync::broadcast::channel(1);
    let mut subscription = ClientPushSubscription(receiver);
    let frame: Arc<str> = Arc::from(r#"{"status":"push"}"#);

    // When / Then
    sender.send(frame.clone()).unwrap();
    assert_eq!(subscription.recv().await.unwrap(), frame);
    sender.send(frame.clone()).unwrap();
    sender.send(frame.clone()).unwrap();
    assert!(matches!(
        subscription.recv().await,
        Err(ClientPushError::Lagged(1))
    ));
    assert_eq!(subscription.recv().await.unwrap(), frame);
    drop(sender);
    assert!(matches!(
        subscription.recv().await,
        Err(ClientPushError::Closed)
    ));
}
