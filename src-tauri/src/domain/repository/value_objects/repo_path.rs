/// リポジトリパスの正規化（値オブジェクトの生成ロジック）。
///
/// - バックスラッシュをスラッシュへ変換
/// - 連続スラッシュを 1 つに畳み込む（UNC プレフィックス `//` は保持）
/// - 末尾スラッシュを除去
pub fn normalize_repo_path(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let had_unc_prefix = replaced.starts_with("//");
    let mut result = String::with_capacity(replaced.len());
    let mut prev_slash = false;
    for c in replaced.chars() {
        if c == '/' {
            if !prev_slash {
                result.push(c);
            }
            prev_slash = true;
        } else {
            result.push(c);
            prev_slash = false;
        }
    }
    let mut normalized = result.trim_end_matches('/').to_string();
    if had_unc_prefix && normalized.starts_with('/') && !normalized.starts_with("//") {
        normalized.insert(0, '/');
    }
    normalized
}

#[cfg(test)]
mod repo_path_tests {
    use super::*;

    #[test]
    fn test_リポジトリパス正規化_末尾スラッシュ除去() {
        assert_eq!(normalize_repo_path("/repo/path/"), "/repo/path");
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
