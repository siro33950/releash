use super::*;

#[test]
fn test_terminal購読_空id重複上限終了を判定し解放後に再利用できる() {
    // Given
    let mut subscriptions = TerminalSubscriptions::default();
    assert_eq!(
        subscriptions.subscribe(" ".into(), ()),
        Err(TerminalSubscriptionError::InvalidId)
    );
    assert_eq!(
        subscriptions.get("missing"),
        Err(TerminalSubscriptionError::Ended)
    );
    // When / Then
    for id in 0..TERMINAL_SUBSCRIPTION_LIMIT {
        subscriptions.subscribe(id.to_string(), ()).unwrap();
    }
    assert_eq!(
        subscriptions.subscribe("0".into(), ()),
        Err(TerminalSubscriptionError::AlreadyExists)
    );
    assert_eq!(
        subscriptions.subscribe("overflow".into(), ()),
        Err(TerminalSubscriptionError::Limit)
    );
    subscriptions.unsubscribe("0");
    assert_eq!(
        subscriptions.get("0"),
        Err(TerminalSubscriptionError::Ended)
    );
    subscriptions.subscribe("0".into(), ()).unwrap();
    for _ in 0..TERMINAL_ATTACHMENT_LIMIT {
        subscriptions.reserve_attachment().unwrap();
    }
    assert_eq!(
        subscriptions.reserve_attachment(),
        Err(TerminalSubscriptionError::AttachmentLimit)
    );
    subscriptions.release_attachment();
    subscriptions.reserve_attachment().unwrap();
}
