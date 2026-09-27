use super::*;

#[tokio::test]
async fn test_終了購読_遅延と取消を内部観測へ伝える() {
    let (sender, _) = tokio::sync::broadcast::channel(1);
    let source = TerminalSurfaceEventSourceGateway::new(sender.clone());
    let mut stream = source.subscribe();
    for sequence in [1, 2] {
        sender
            .send(TerminalSurfaceEvent::Exit {
                session_key: "session".into(),
                runtime_generation: 1,
                exit_code: Some(0),
                sequence,
            })
            .unwrap();
    }

    assert!(matches!(
        stream.subscription.recv().await,
        Err(TerminalSurfaceEventReceiveError::Lagged(1))
    ));
    assert!(matches!(
        stream.subscription.recv().await,
        Ok(TerminalSurfaceEvent::Exit { sequence: 2, .. })
    ));
    stream.cancellation.cancel();
    assert!(matches!(
        stream.subscription.recv().await,
        Err(TerminalSurfaceEventReceiveError::Closed)
    ));
}
