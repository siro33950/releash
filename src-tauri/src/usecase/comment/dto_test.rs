use super::*;

#[test]
fn test_review停止_詳細payloadに停止のコードを返す() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for (stopped, code) in [
        (OperationStopped::Expired, "expired"),
        (OperationStopped::Cancelled, "cancelled"),
    ] {
        // When
        let json = review_error_to_json_string(domain::ReviewError::Technical(stopped.into()));
        // Then
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["code"], code);
    }
}
