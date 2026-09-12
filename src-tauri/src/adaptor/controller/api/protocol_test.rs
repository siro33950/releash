use super::*;

#[test]
fn test_汎用応答_成功と失敗は排他的でrequest_idを保持する() {
    // Given / When
    let success = CommandResponse::new("request-1".into(), Ok(Value::Null));
    let failure = CommandResponse::new(
        "request-2".into(),
        Err(crate::other::AppError::coded("INVALID_REQUEST", "invalid")),
    );
    // Then
    assert_eq!(
        serde_json::to_value(success).unwrap(),
        serde_json::json!({"request_id":"request-1","result":null})
    );
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        serde_json::json!({"request_id":"request-2","error":{"code":"INVALID_REQUEST","message":"invalid"}})
    );
}

#[test]
fn test_stream規約_attachmentごとのsequenceと累積ackを表現する() {
    // Given
    let frames = [
        serde_json::json!({"type":"stream","attachment_id":"a","sequence":1,"data":"日本語"}),
        serde_json::json!({"type":"stream","attachment_id":"b","sequence":1,"data":"other"}),
        serde_json::json!({"type":"ack","attachment_id":"a","sequence":1}),
    ];
    // When / Then
    for frame in frames {
        let envelope: StreamEnvelope = serde_json::from_value(frame.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), frame);
    }
    assert_eq!(MAX_STREAM_FRAME_BYTES, 65536);
    assert!(serde_json::from_value::<StreamEnvelope>(
        serde_json::json!({"type":"ack","sequence":1})
    )
    .is_err());
}
