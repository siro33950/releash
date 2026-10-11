use super::*;

#[test]
fn test_旧表示状態の移行先_hashキーを優先し末尾名が一意なworktreeだけ選ぶ() {
    // Given
    let candidates = vec![
        ("/repo/feature/a".into(), "hash-one".into()),
        ("/repo/bugfix/a".into(), "hash-two".into()),
        ("/repo/unique".into(), "hash-three".into()),
    ];
    // When / Then
    assert_eq!(
        legacy_state_path("hash-one", &candidates),
        Some("/repo/feature/a")
    );
    assert_eq!(
        legacy_state_path("unique", &candidates),
        Some("/repo/unique")
    );
    assert_eq!(legacy_state_path("a", &candidates), None);
    assert_eq!(legacy_state_path("missing", &candidates), None);
    assert_eq!(legacy_state_path("unique", &[]), None);
    assert_eq!(
        legacy_state_path("_repo_feature_a", &candidates),
        Some("/repo/feature/a")
    );
    let ambiguous = vec![
        ("/repo/a/b".into(), "one".into()),
        ("/repo/a_b".into(), "two".into()),
    ];
    assert_eq!(legacy_state_path("_repo_a_b", &ambiguous), None);
}

#[test]
fn test_旧表示状態の移行先_未解決repositoryがあれば名前の解決を見送る() {
    // Given / When / Then
    assert_eq!(
        migration_worktree_paths(vec![
            Some(vec!["/repo/a".into()]),
            Some(vec!["/other/b".into()])
        ]),
        vec!["/repo/a", "/other/b"]
    );
    assert!(migration_worktree_paths(vec![Some(vec!["/repo/a".into()]), None]).is_empty());
}
