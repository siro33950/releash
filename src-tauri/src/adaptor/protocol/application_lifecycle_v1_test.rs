use super::*;

#[test]
fn test_起動失敗quit_応答はacceptedとcorrelation_idだけの固定jsonになる() {
    // Given
    let outcome = StartupFailureQuitOutcomeDtoV1::Accepted {
        correlation_id: "startup-correlation".to_string(),
    };

    // When
    let value = serde_json::to_value(outcome).expect("serialize startup failure Quit result");

    // Then
    assert_eq!(
        value,
        serde_json::json!({
            "type": "accepted",
            "correlationId": "startup-correlation",
        })
    );
}
