use super::*;

#[test]
fn test_archive事実codec_旧形式を境界で補い理由と精密な時刻を往復する() {
    // Given / When
    let legacy = crate::adaptor::gateway::workflow::fact_log::decode_stored_fact(
        "archive_requested",
        "{}",
        42125,
    )
    .unwrap()
    .unwrap();
    // Then
    assert_eq!(
        legacy,
        NodeFact::ArchiveRequested(ArchiveRequestedFact {
            reason: "manual".into(),
            archived_at: 42.125
        })
    );
    for reason in ["manual", "worktree_removed"] {
        let fact = NodeFact::ArchiveRequested(ArchiveRequestedFact {
            reason: reason.into(),
            archived_at: 12.345678,
        });
        let detail = fact.encode_detail().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap(),
            serde_json::json!({ "reason": reason, "archivedAt": 12.345678 })
        );
        assert_eq!(
            NodeFact::decode("archive_requested", &detail).unwrap(),
            fact
        );
    }
    for invalid in [
        "null",
        "[]",
        "invalid",
        "{\"reason\":false}",
        "{\"archivedAt\":\"bad\"}",
    ] {
        assert!(NodeFact::decode("archive_requested", invalid).is_err());
    }
}

#[test]
fn test_repository所属の観測事実_往復し欠損と空のpathを拒否する() {
    // Given
    let fact = NodeFact::RepositoryRootObserved("/repo".into());
    // When / Then
    let detail = fact.encode_detail().unwrap();
    assert_eq!(NodeFact::decode(fact.event_type(), &detail).unwrap(), fact);
    for invalid in [
        "{}",
        "null",
        "[]",
        r#"{"repositoryRoot":null}"#,
        r#"{"repositoryRoot":1}"#,
        r#"{"repositoryRoot":" "}"#,
    ] {
        assert!(NodeFact::decode(fact.event_type(), invalid).is_err());
    }
}
