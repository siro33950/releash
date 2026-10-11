use super::*;

#[test]
fn test_旧表示状態の移行先_末尾名が一意なworktreeだけを選ぶ() {
    // Given
    let paths = vec![
        "/repo-worktrees/feature/a".into(),
        "/repo-worktrees/bugfix/a".into(),
        "/repo-worktrees/unique".into(),
    ];
    // When / Then
    assert_eq!(
        legacy_worktree_path("unique", &paths),
        Some("/repo-worktrees/unique")
    );
    assert_eq!(legacy_worktree_path("a", &paths), None);
    assert_eq!(legacy_worktree_path("missing", &paths), None);
    assert_eq!(legacy_worktree_path("unique", &[]), None);
    assert_eq!(legacy_worktree_path("_repo_b", &["/repo/b".into()]), None);
    assert_eq!(
        legacy_worktree_path("_repo_a", &["/repo/_repo_a".into(), "/repo/a".into()]),
        Some("/repo/_repo_a")
    );
    assert_eq!(
        legacy_worktree_path("_repo_a_b", &["/repo/a/b".into(), "/repo/a_b".into()]),
        None
    );
}
