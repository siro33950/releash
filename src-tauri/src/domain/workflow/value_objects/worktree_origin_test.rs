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
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn canonical_identity_uses_sibling_isolated_directory_and_branch() {
        assert_eq!(
            isolated_worktree_path("/projects/repo", "node-1", 2),
            "/projects/repo-worktrees/.releash-isolated/node-1-a2"
        );
        assert_eq!(
            isolated_worktree_branch("node-1", 2),
            "releash/isolated/node-1-a2"
        );
    }

    #[test]
    fn fallback_requires_matching_canonical_path_and_branch_token() {
        let canonical = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/.releash-isolated/node-1-a2",
            "releash/isolated/node-1-a2",
        );
        assert!(canonical.matches_isolated_identity_rule());

        let wrong_branch = WorktreeInventoryEntry::new(
            "/projects/repo",
            &canonical.worktree_path,
            "feature/node-1-a2",
        );
        assert!(!wrong_branch.matches_isolated_identity_rule());

        let wrong_path = WorktreeInventoryEntry::new(
            "/projects/repo",
            "/projects/repo-worktrees/node-1-a2",
            &canonical.branch,
        );
        assert!(!wrong_path.matches_isolated_identity_rule());
    }
}
