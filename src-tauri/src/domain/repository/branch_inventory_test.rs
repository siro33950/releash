use super::*;

#[test]
fn test_ブランチ分類_隔離を除外し通常worktreeと未配置ブランチを区別する() {
    // Given
    let mut cards = vec![
        ("main", Some("/repo")),
        ("feature", None),
        (
            "releash/isolated/orphan-a1",
            Some("/repo-worktrees/.releash-isolated/orphan-a1"),
        ),
    ];
    // When
    let working = classify_branch_cards("/repo", &mut cards, |card| *card);
    // Then
    assert_eq!(cards, vec![("main", Some("/repo")), ("feature", None)]);
    assert_eq!(working, vec![("main", Some("/repo"))]);
}
