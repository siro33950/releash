mod branch_tests {
    use super::super::*;

    #[test]
    fn test_base候補_リモートと指定したbranchを除外する() {
        assert!(Branch::local("main").is_base_candidate(Some("feature")));
        assert!(!Branch::local("main").is_base_candidate(Some("main")));
        assert!(!Branch::remote("origin/main").is_base_candidate(None));
    }

    #[test]
    fn test_worktreeのbase候補_ローカルとoriginのリモートだけ選べる() {
        // Given / When / Then
        assert!(Branch::local("main").is_worktree_base());
        assert!(Branch::remote("origin/main").is_worktree_base());
        assert!(!Branch::remote("upstream/main").is_worktree_base());
    }

    #[test]
    fn test_ブランチ生成_ローカルとリモートの区別() {
        assert!(!Branch::local("main").is_remote);
        assert!(Branch::remote("feature").is_remote);
    }
    #[test]
    fn test_worktree作成候補_ローカルでworktreeを持たないbranchだけ選べる() {
        assert!(Branch::local("feature").can_create_worktree(false));
        assert!(!Branch::local("feature").can_create_worktree(true));
        assert!(!Branch::remote("origin/feature").can_create_worktree(false));
    }
}

#[test]
fn test_ブランチ作成_ローカルは再利用し未存在とremoteは新規にする() {
    // Given
    let branches = vec![
        (super::Branch::local("existing"), false),
        (super::Branch::remote("origin/main"), false),
    ];
    // When / Then
    assert!(!super::Branch::needs_creation("existing", &branches));
    assert!(super::Branch::needs_creation("new", &branches));
    assert!(super::Branch::needs_creation("origin/main", &branches));
}

#[test]
fn test_origin名_originのリモートだけを返す() {
    // Given / When / Then
    assert_eq!(
        super::Branch::remote("origin/topic").origin_name(),
        Some("topic")
    );
    assert_eq!(super::Branch::local("origin/topic").origin_name(), None);
    assert_eq!(super::Branch::remote("upstream/topic").origin_name(), None);
}
