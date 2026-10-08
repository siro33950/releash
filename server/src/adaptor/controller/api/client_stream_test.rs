use super::*;

#[test]
fn test_購読識別子_長さ上限まで受け付け空文字と超過を拒否する() {
    // Given
    let at_limit = "x".repeat(SUBSCRIPTION_ID_MAX_BYTES);
    let over_limit = "x".repeat(SUBSCRIPTION_ID_MAX_BYTES + 1);
    // When
    let accepted = [
        valid_subscription_id(&at_limit),
        valid_subscription_id(&over_limit),
        valid_subscription_id(""),
    ];
    // Then
    assert_eq!(accepted, [true, false, false]);
}
