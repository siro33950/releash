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
