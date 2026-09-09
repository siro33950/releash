use super::*;
use crate::domain::workflow::WorktreeMode;

#[test]
fn test_worktree継承_省略とsharedは親を継承しisolatedは自身のattemptを使う() {
    // Given
    let root = "/repo";
    let parent_rule = WorktreeInheritance::new(Some(WorktreeMode::Isolated));
    let parent = parent_rule.for_attempt(Some(root), "parent", 1).unwrap();
    for mode in [
        None,
        Some(WorktreeMode::Shared),
        Some(WorktreeMode::Isolated),
    ] {
        let rule = WorktreeInheritance::new(mode);
        // When
        let own = rule.for_attempt(Some(root), "child", 2).unwrap();
        let path = WorktreeInheritance::effective_path(
            root,
            [(rule, own.as_ref()), (parent_rule, parent.as_ref())],
        );
        // Then
        assert_eq!(rule.is_isolated(), mode == Some(WorktreeMode::Isolated));
        assert_eq!(path, own.as_ref().or(parent.as_ref()).unwrap().path);
        if mode != Some(WorktreeMode::Isolated) {
            assert_eq!(rule.for_attempt(None, "child", 2).unwrap(), None);
            assert_eq!(
                WorktreeInheritance::effective_path(root, [(rule, None)]),
                root
            );
        }
    }
    assert!(parent_rule.for_attempt(None, "parent", 1).is_err());
}

#[test]
fn test_隔離成果_提出値の有無によらず所有attemptのworktreeを合成する() {
    // Given
    let worktree = IsolatedWorktree::for_attempt("/repo", "owner", 2);
    // When / Then
    assert_eq!(
        worktree.with_artifact(None),
        serde_json::json!({"worktree": {"branch": worktree.branch, "path": worktree.path}})
    );
    assert_eq!(
        worktree.with_artifact(Some(
            serde_json::json!({"result": true, "worktree": {"path": "wrong"}})
        )),
        serde_json::json!({"result": true, "worktree": {"branch": worktree.branch, "path": worktree.path}})
    );
}
