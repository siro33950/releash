//! 層をまたぐ汎用ユーティリティ（特定ドメインに属さない純粋関数）。

/// 現在時刻を Unix epoch からの秒数として返す。
///
/// システム時刻が epoch より前などで変換できない場合は、既存呼び出し元の挙動に合わせて
/// `0.0` を返す。
pub fn unix_timestamp_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

/// `base` ディレクトリを基準に `path` を相対化する。
///
/// `path` が `base/` 接頭辞を持つ場合はそれを除去した相対パスを、持たない場合は `path`
/// をそのまま返す。外部 I/O を持たない純粋なパス文字列操作で、基準パスをハードコード
/// せず引数で受け取る。`code` / `repository` いずれのドメイン知識も含まないため、特定
/// ドメインサービスではなく層に属さない横断ユーティリティとして配置する。
pub fn relative_path(base: &str, path: &str) -> Option<String> {
    let prefix = format!("{base}/");
    if path.starts_with(&prefix) {
        Some(path[prefix.len()..].to_string())
    } else {
        Some(path.to_string())
    }
}

#[cfg(test)]
mod utils_tests {
    use super::*;

    #[test]
    fn test_相対化_base配下は接頭辞を除去する() {
        let result = relative_path("/Users/foo/project", "/Users/foo/project/src/index.ts");
        assert_eq!(result, Some("src/index.ts".to_string()));
    }

    #[test]
    fn test_相対化_base外のパスはそのまま返す() {
        let result = relative_path("/Users/foo/project", "other/path.ts");
        assert_eq!(result, Some("other/path.ts".to_string()));
    }
}
