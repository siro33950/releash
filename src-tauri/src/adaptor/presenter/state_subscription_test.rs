#[test]
fn test_配信失敗_usecaseの失敗分類へ意味を保って変換する() {
    use crate::infrastructure::state_subscription::SubscriptionError as DeliveryError;
    use crate::usecase::state_subscription::SubscriptionError;

    let cases = [
        (DeliveryError::InvalidId, SubscriptionError::InvalidId),
        (
            DeliveryError::AlreadyExists,
            SubscriptionError::AlreadyExists,
        ),
        (DeliveryError::StreamEnded, SubscriptionError::StreamEnded),
        (
            DeliveryError::UnknownTarget,
            SubscriptionError::UnknownTarget,
        ),
        (
            DeliveryError::VersionExhausted,
            SubscriptionError::VersionExhausted,
        ),
        (
            DeliveryError::SnapshotRequired,
            SubscriptionError::SnapshotRequired,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(SubscriptionError::from(source), expected);
    }
}
