use super::*;

#[test]
fn test_失敗出力_messageなしではjsonと平文の既存の代替値を保つ() {
    // Given
    let mut error = connectrpc::ConnectError::unavailable("unused");
    error.message = None;
    for guidance in [None, Some(client::startup_guidance())] {
        // When
        let machine: serde_json::Value =
            serde_json::from_str(&failure_output(&error, true, guidance)).unwrap();
        let human = failure_output(&error, false, guidance);
        // Then
        let mut expected = serde_json::json!({"error": {
            "code": "unavailable", "message": error.to_string(),
        }});
        let mut expected_human = "error: unavailable: Request failed\n".to_string();
        if let Some(guidance) = guidance {
            expected["error"]["guidance"] = serde_json::json!(guidance);
            expected_human.push_str(&format!("{guidance}\n"));
        }
        assert_eq!(machine, expected);
        assert_eq!(human, expected_human);
    }
}
