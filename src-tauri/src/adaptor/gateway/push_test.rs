use super::*;

#[tokio::test]
async fn test_push購読_frameと欠落と終了をgateway境界で返す() {
    // Given
    let (sender, receiver) = tokio::sync::broadcast::channel(1);
    let mut subscription = ClientPushSubscription(receiver);
    let frame: Arc<[u8]> = Arc::from(&[1u8, 2, 3][..]);

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

#[tokio::test]
async fn test_agent_session通知_購読対象の更新を通知する() {
    use crate::usecase::agent_session::AgentSessionChangeNotifier;
    let publisher = crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
    let mut changes = publisher.subscribe_changes();
    let notifier = ClientAgentSessionChangeNotifier::new(publisher);
    notifier.agent_session_changed("/repo");
    assert_eq!(
        changes.recv().await.unwrap(),
        crate::domain::state_subscription::StateChangeSource::Worktree("/repo".into())
    );
}
