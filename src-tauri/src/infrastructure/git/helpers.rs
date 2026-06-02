//! git2 を直接扱う共通プリミティブ（neutral な infrastructure 層）。
//!
//! repository ドメインの各 gateway 実装に加え、未移行の code ドメイン
//! （`git/diff.rs`・`git/branch_diff.rs`）からも参照される。code ドメイン →
//! adaptor/gateway の逆方向依存を避けるため、最下層の infrastructure に配置する。
//!
//! ここに置くのは git2 の単純な問い合わせに閉じたプリミティブのみ。複数情報源を
//! 合成する「ベースブランチ解決順序」のような業務ルールは infrastructure の責務外
//! であり、利用側のレイヤー（gateway / code ドメイン）が `detect_default_branch`
//! などのプリミティブを組み合わせて構成する。

use git2::{BranchType, ErrorCode, Repository};

/// 既定ブランチ名を検出する。
/// remote HEAD（`refs/remotes/origin/HEAD`）を最優先、次に `main` / `master`。
pub(crate) fn detect_default_branch(repo: &Repository) -> Option<String> {
    // remote HEAD (refs/remotes/origin/HEAD) を最優先で確認
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(resolved) = reference.resolve() {
            if let Ok(name) = resolved.shorthand() {
                // "origin/main" → "main"
                let short = name.strip_prefix("origin/").unwrap_or(name);
                if repo.find_branch(short, BranchType::Local).is_ok() {
                    return Some(short.to_string());
                }
            }
        }
    }

    for name in &["main", "master"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Some(name.to_string());
        }
    }
    None
}

/// リポジトリの HEAD が指すブランチ名（detached / unborn は表示用文字列）。
pub(crate) fn get_branch_name_for_repo(repo: &Repository) -> String {
    match repo.head() {
        Ok(head) => {
            if head.is_branch() {
                head.shorthand().unwrap_or("HEAD").to_string()
            } else {
                let oid = head.target().map(|o| o.to_string());
                match oid {
                    Some(h) => format!("({})", &h[..7.min(h.len())]),
                    None => "HEAD".to_string(),
                }
            }
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch => "(no commits)".to_string(),
        Err(_) => "unknown".to_string(),
    }
}
