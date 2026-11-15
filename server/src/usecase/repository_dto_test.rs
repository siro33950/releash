use super::*;
#[test]
fn test_走査結果_競合種別の転送形式と復元を保つ() {
    use crate::domain::git::ConflictKind;
    // Given
    for (kind, name) in [
        (ConflictKind::BothModified, "both_modified"),
        (ConflictKind::BothAdded, "both_added"),
        (ConflictKind::BothDeleted, "both_deleted"),
        (ConflictKind::AddedByUs, "added_by_us"),
        (ConflictKind::AddedByThem, "added_by_them"),
        (ConflictKind::DeletedByUs, "deleted_by_us"),
        (ConflictKind::DeletedByThem, "deleted_by_them"),
    ] {
        let dto = FileStatusDto {
            path: "a".into(),
            index_status: "modified".into(),
            worktree_status: "modified".into(),
            conflict: Some(kind),
        };
        // When
        let value = serde_json::to_value(&dto).unwrap();
        let restored: FileStatusDto = serde_json::from_value(value.clone()).unwrap();
        // Then
        assert_eq!(value["conflict"], name);
        assert_eq!(FileStatus::from(restored).conflict, Some(kind));
    }
    let dto = FileStatusDto {
        path: "a".into(),
        index_status: "none".into(),
        worktree_status: "new".into(),
        conflict: None,
    };
    assert_eq!(FileStatus::from(dto).conflict, None);
}
