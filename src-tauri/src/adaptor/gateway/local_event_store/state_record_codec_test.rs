use super::*;

#[test]
fn test_復旧結果のdigest_名前順のjsonバイト列との互換性を保つ() {
    // Given
    let canonical = br#"{"classification":"succeeded","outcome":"terminal","resource_revision":2,"resource_view":"{\"state\":\"completed\"}","schema":"recovery_action_canonical_result_v1"}"#;
    let expected: [u8; 32] = Sha256::digest(canonical).into();

    // When
    let digest = canonical_recovery_result_sha256(
        RecoveryResultOutcomeRecord::Terminal,
        RecoveryResultClassification::Succeeded,
        2,
        r#"{"state":"completed"}"#,
    )
    .unwrap();

    // Then
    assert_eq!(digest, expected);
}

#[test]
fn test_復旧結果の復元_保存済みrecordと完了payloadのhashが一致する() {
    // Given
    let saved = r#"{"canonical_result_sha256":"110614ee33673b7076e57cf827ab8bb18ee5eb83ffc0c6c50a6a748880f85ccf","classification":"succeeded","outcome":"terminal","resource_revision":2,"resource_view":"{\"state\":\"completed\"}","schema":"recovery_action_result_v1"}"#;

    // When
    let decoded = StoredRecoveryResultV1::decode(saved).unwrap().into_value();
    let finish_payload = canonicalize_recovery_result_record(
        RecoveryResultOutcomeRecord::Terminal,
        RecoveryResultClassification::Succeeded,
        2,
        RecoveryResourceViewRecord::SafeSummary(r#"{"state":"completed"}"#.to_string()),
    )
    .unwrap();

    // Then
    assert_eq!(finish_payload, decoded);
    let RecoveryResultRecord::Action(result) = finish_payload;
    assert_eq!(
        hex::encode(result.canonical_result_sha256),
        "110614ee33673b7076e57cf827ab8bb18ee5eb83ffc0c6c50a6a748880f85ccf"
    );
}
