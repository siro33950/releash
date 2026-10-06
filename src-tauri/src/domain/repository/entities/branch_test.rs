mod branch_tests {
    use super::super::*;

    #[test]
    fn test_base候補_リモートと指定したbranchを除外する() {
        assert!(Branch::local("main").is_base_candidate(Some("feature")));
        assert!(!Branch::local("main").is_base_candidate(Some("main")));
        assert!(!Branch::remote("origin/main").is_base_candidate(None));
    }

    #[test]
    fn test_ブランチ生成_ローカルとリモートの区別() {
        assert!(!Branch::local("main").is_remote);
        assert!(Branch::remote("feature").is_remote);
    }
}
