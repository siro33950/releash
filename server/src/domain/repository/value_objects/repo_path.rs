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
    // A filesystem root is already canonical. Trimming its only separator would turn
    // `/` into an empty identity (and `C:/` into `C:`), which is unsafe once this value
    // is used for worktree/session ownership comparisons.
    let is_posix_root = result == "/";
    let is_windows_drive_root =
        result.len() == 3 && result.as_bytes()[1] == b':' && result.as_bytes()[2] == b'/';
    if is_posix_root || is_windows_drive_root {
        return result;
    }

    let mut normalized = result.trim_end_matches('/').to_string();
    if had_unc_prefix && normalized.starts_with('/') && !normalized.starts_with("//") {
        normalized.insert(0, '/');
    }
    normalized
}

#[cfg(test)]
#[path = "repo_path_test.rs"]
mod repo_path_tests;
