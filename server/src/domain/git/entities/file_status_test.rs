use super::*;
#[test]
fn test_変更分類_状態と競合を区別する() {
    // Given / When / Then
    for (index, worktree, staged, unstaged) in [
        (
            "new",
            "new",
            Some(ChangeKind::Added),
            Some(ChangeKind::Untracked),
        ),
        (
            "deleted",
            "deleted",
            Some(ChangeKind::Deleted),
            Some(ChangeKind::Deleted),
        ),
        (
            "renamed",
            "modified",
            Some(ChangeKind::Modified),
            Some(ChangeKind::Modified),
        ),
        ("modified", "ignored", Some(ChangeKind::Modified), None),
        ("none", "none", None, None),
    ] {
        let mut file = FileStatus {
            path: "a".into(),
            index_status: index.into(),
            worktree_status: worktree.into(),
            conflict: None,
        };
        assert_eq!(file.staged_change(), staged);
        assert_eq!(file.unstaged_change(), unstaged);
        file.conflict = Some(ConflictKind::BothModified);
        assert_eq!(file.staged_change(), None);
        assert_eq!(file.unstaged_change(), None);
    }
}
#[test]
fn test_競合種別_往復と未知値の拒否() {
    // Given / When / Then
    for (kind, name) in [
        (ConflictKind::BothModified, "both_modified"),
        (ConflictKind::BothAdded, "both_added"),
        (ConflictKind::BothDeleted, "both_deleted"),
        (ConflictKind::AddedByUs, "added_by_us"),
        (ConflictKind::AddedByThem, "added_by_them"),
        (ConflictKind::DeletedByUs, "deleted_by_us"),
        (ConflictKind::DeletedByThem, "deleted_by_them"),
    ] {
        assert_eq!(kind.as_str(), name);
        assert_eq!(ConflictKind::parse(name), Ok(kind));
    }
    assert!(ConflictKind::parse("unknown").is_err());
}
