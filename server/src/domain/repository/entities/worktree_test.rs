use super::*;

#[test]
fn test_worktree削除可否_mainとlockedとdirtyを副作用前に拒否する() {
    // Given
    let mut worktree = Worktree {
        name: "wt".into(),
        path: "/wt".into(),
        branch: "feat".into(),
        is_main: false,
        is_locked: false,
        is_merged: false,
    };
    // When / Then
    assert!(worktree.authorize_removal(false, 0).is_ok());
    assert!(worktree.authorize_removal(false, 1).is_err());
    assert!(worktree.authorize_removal(true, 1).is_ok());
    worktree.is_locked = true;
    assert!(worktree.authorize_removal(false, 0).is_err());
    assert!(worktree.authorize_removal(true, 0).is_ok());
    worktree.is_main = true;
    assert!(worktree.authorize_removal(true, 0).is_err());
}

#[test]
fn test_削除確認_lockedまたは変更があるときだけ強制を求める() {
    // Given
    let mut worktree = Worktree::being_deleted("/wt", "feat".into());
    // When / Then
    assert_eq!(worktree.removal_requires_force(Some(0)), Some(false));
    assert_eq!(worktree.removal_requires_force(None), None);
    assert_eq!(worktree.removal_requires_force(Some(1)), Some(true));
    worktree.is_locked = true;
    assert_eq!(worktree.removal_requires_force(Some(0)), Some(true));
    assert_eq!(worktree.removal_requires_force(None), Some(true));
}
