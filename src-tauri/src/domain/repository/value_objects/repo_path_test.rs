mod repo_path_tests {
    use super::super::*;

    #[test]
    fn test_リポジトリパス正規化_末尾スラッシュ除去() {
        assert_eq!(normalize_repo_path("/repo/path/"), "/repo/path");
    }

    #[test]
    fn test_リポジトリパス正規化_ルートは空文字にしない() {
        assert_eq!(normalize_repo_path("/"), "/");
        assert_eq!(normalize_repo_path("C:\\"), "C:/");
    }

    #[test]
    fn test_リポジトリパス正規化_バックスラッシュ変換() {
        assert_eq!(
            normalize_repo_path("C:\\Users\\test\\repo"),
            "C:/Users/test/repo"
        );
    }

    #[test]
    fn test_リポジトリパス正規化_連続スラッシュ畳み込み() {
        assert_eq!(normalize_repo_path("/repo//path///sub"), "/repo/path/sub");
    }

    #[test]
    fn test_リポジトリパス正規化_複合() {
        assert_eq!(
            normalize_repo_path("C:\\Users\\\\test\\repo/"),
            "C:/Users/test/repo"
        );
    }

    #[test]
    fn test_リポジトリパス正規化_unc_プレフィックス保持() {
        assert_eq!(
            normalize_repo_path("\\\\server\\share\\repo"),
            "//server/share/repo"
        );
    }

    #[test]
    fn test_リポジトリパス正規化_unc_と連続スラッシュ() {
        assert_eq!(
            normalize_repo_path("\\\\server\\share\\\\repo"),
            "//server/share/repo"
        );
    }
}
