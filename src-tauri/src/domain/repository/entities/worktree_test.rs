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
