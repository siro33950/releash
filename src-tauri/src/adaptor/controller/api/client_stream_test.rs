use super::*;

#[test]
fn test_購読識別子_二入口で同じ長さ上限を使い空文字の扱いだけを分ける() {
    // Given
    let at_limit = "x".repeat(128);
    let over_limit = "x".repeat(129);
    // When
    let accepted = [
        valid_subscription_id(&at_limit, true),
        valid_subscription_id(&at_limit, false),
        valid_subscription_id(&over_limit, true),
        valid_subscription_id(&over_limit, false),
        valid_subscription_id("", true),
        valid_subscription_id("", false),
    ];
    // Then
    assert_eq!(accepted, [true, true, false, false, true, false]);
}
