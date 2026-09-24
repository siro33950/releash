use super::*;

#[test]
fn test_監視購読_所有と受理上限は購読終了と停止で解放する() {
    // Given
    let mut subscriptions = WatchSubscriptions::default();
    assert_eq!(
        subscriptions.ensure_can_watch("missing"),
        Err(WatchSubscriptionError::NotFound)
    );
    subscriptions.subscribe("push".into()).unwrap();
    assert_eq!(
        subscriptions.subscribe("push".into()),
        Err(WatchSubscriptionError::AlreadyExists)
    );
    // When
    for id in 0..64 {
        let reservation = subscriptions.reserve("push").unwrap();
        subscriptions.complete(reservation, Some(id)).unwrap();
    }
    // Then
    assert_eq!(
        subscriptions.reserve("push").map(|_| ()),
        Err(WatchSubscriptionError::Limit)
    );
    subscriptions.stopped(0);
    let reservation = subscriptions.reserve("push").unwrap();
    subscriptions.complete(reservation, Some(64)).unwrap();
    assert_eq!(subscriptions.unsubscribe("push"), (1..=64).collect());
    assert_eq!(
        subscriptions.ensure_can_watch("push"),
        Err(WatchSubscriptionError::NotFound)
    );
    for id in 0..16 {
        subscriptions.subscribe(id.to_string()).unwrap();
    }
    assert_eq!(
        subscriptions.subscribe("overflow".into()),
        Err(WatchSubscriptionError::Limit)
    );
    subscriptions.unsubscribe("0");
    subscriptions.subscribe("next".into()).unwrap();
}

#[test]
fn test_監視予約_生成中も上限を保ち失敗と旧購読の予約を解放する() {
    // Given
    let mut subscriptions = WatchSubscriptions::default();
    subscriptions.subscribe("push".into()).unwrap();
    let mut pending: Vec<_> = (0..64)
        .map(|_| subscriptions.reserve("push").unwrap())
        .collect();
    // When / Then
    assert_eq!(
        subscriptions.reserve("push").err(),
        Some(WatchSubscriptionError::Limit)
    );
    subscriptions
        .complete(pending.pop().unwrap(), None)
        .unwrap();
    let replacement = subscriptions.reserve("push").unwrap();
    subscriptions.unsubscribe("push");
    subscriptions.subscribe("push".into()).unwrap();
    assert_eq!(
        subscriptions.complete(replacement, Some(42)),
        Err(WatchSubscriptionError::NotFound)
    );
    assert!(subscriptions.unsubscribe("push").is_empty());
}

#[test]
fn test_失敗分類_watch_subscription_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (WatchSubscriptionError::NotFound, F::Missing),
        (WatchSubscriptionError::AlreadyExists, F::AlreadyPresent),
        (WatchSubscriptionError::Limit, F::Capacity),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
