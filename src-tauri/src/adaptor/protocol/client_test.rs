use super::*;

#[test]
fn test_クライアント接続情報_renderer向けのjson_field名を保持する() {
    // Given
    let endpoint = ClientEndpoint {
        url: "ws://127.0.0.1:123/v1/client".into(),
        auth_subprotocol: "releash-bearer.client".into(),
    };
    // When / Then
    assert_eq!(
        serde_json::to_value(endpoint).unwrap(),
        serde_json::json!({
            "url": "ws://127.0.0.1:123/v1/client",
            "authSubprotocol": "releash-bearer.client",
        })
    );
}
