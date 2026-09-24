use super::*;

#[test]
fn test_snapshot公開形式_limitedを含まずstaleとloadingを保持する() {
    // Given
    let snapshot = RepositorySnapshot::loading().with_read_flags(true, true);

    // When
    let values = [
        serde_json::to_value(&snapshot.flags).unwrap(),
        serde_json::to_value(RepositoryBranchCardsSnapshotDto::from_snapshot(&snapshot)).unwrap(),
    ];

    // Then
    for value in values {
        assert!(value.get("limited").is_none());
        assert_eq!(value["stale"], true);
        assert_eq!(value["loading"], true);
    }
}
