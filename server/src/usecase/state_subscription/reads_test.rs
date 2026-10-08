use super::*;

#[test]
fn test_状態読取失敗_購読失敗の種類と表示を保つ() {
    let failure = crate::usecase::state_subscription::SubscriptionError::UnknownTarget;
    let error = StateReadError::from_error(failure);

    assert!(matches!(
        error.source,
        StateReadFailure::Subscription(ref inner) if **inner == failure
    ));
    assert_eq!(error.to_string(), "UnknownTarget");
}
