use super::*;
use prost::Message;

#[test]
fn test_起動失敗_利用不可エラーの意味をwireの往復で保持する() {
    // Given
    let error: wire::CommandError =
        crate::usecase::application_startup::ApplicationUnavailable::ApplicationUnavailable.into();
    // When
    let decoded = wire::CommandError::decode(error.encode_to_vec().as_slice()).unwrap();
    // Then
    assert_eq!(
        wire::from_value(decoded).unwrap(),
        serde_json::json!({"type": "application_unavailable"})
    );
}
