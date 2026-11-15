use super::*;
#[test]
fn test_走査結果_競合種別を復元し不明な種別を拒否する() {
    // Given
    let mut dto = FileStatusDto {
        path: "a".into(),
        index_status: "modified".into(),
        worktree_status: "modified".into(),
        conflict: Some("both_added".into()),
    };
    // When / Then
    assert_eq!(
        FileStatus::try_from(dto.clone()).unwrap().conflict,
        Some(crate::domain::git::ConflictKind::BothAdded)
    );
    dto.conflict = None;
    assert_eq!(FileStatus::try_from(dto.clone()).unwrap().conflict, None);
    dto.conflict = Some("invalid".into());
    assert!(FileStatus::try_from(dto).is_err());
}
