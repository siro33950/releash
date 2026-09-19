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
async fn test_agent_session通知_gatewayが共有sinkへprotoの変更通知を送る() {
    use crate::adaptor::protocol::client as wire;
    use crate::usecase::agent_session::AgentSessionChangeNotifier;
    use prost::Message;
    // Given
    let sink = Arc::new(PushSink::new());
    let mut subscription = ClientPushGateway::new(sink.clone()).subscribe();
    let notifier = ClientAgentSessionChangeNotifier::new(sink);
    // When
    notifier.agent_session_changed("/repo");
    let frame = subscription.recv().await.unwrap();
    // Then
    let push = wire::Push::decode(frame.as_ref()).unwrap();
    let Some(wire::push::Event::AgentSessionChanged(event)) = push.event else {
        panic!("agent session push")
    };
    assert_eq!(event.worktree_path.as_deref(), Some("/repo"));
}
