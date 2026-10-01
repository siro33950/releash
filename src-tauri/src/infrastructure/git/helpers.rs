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
pub(crate) fn detect_default_branch<E: From<git2::Error>>(
    repo: &Repository,
    check: &dyn Fn() -> Result<(), E>,
) -> Result<Option<String>, E> {
    check()?;
    let optional = |result: Result<git2::Branch<'_>, git2::Error>| match result {
        Ok(_) => Ok(true),
        Err(error) if error.code() == ErrorCode::NotFound => Ok(false),
        Err(error) => Err(E::from(error)),
    };
    match repo.find_reference("refs/remotes/origin/HEAD") {
        Ok(reference) => {
            check()?;
            let resolved = match reference.resolve() {
                Ok(reference) => Some(reference),
                Err(error) if error.code() == ErrorCode::NotFound => None,
                Err(error) => return Err(E::from(error)),
            };
            check()?;
            if let Some(resolved) = resolved {
                let name = resolved.shorthand().map_err(E::from)?;
                let short = name.strip_prefix("origin/").unwrap_or(name);
                check()?;
                if optional(repo.find_branch(short, BranchType::Local))? {
                    return Ok(Some(short.to_string()));
                }
            }
        }
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return Err(E::from(error)),
    }
    for name in ["main", "master"] {
        check()?;
        let found = optional(repo.find_branch(name, BranchType::Local))?;
        check()?;
        if found {
            return Ok(Some(name.to_string()));
        }
    }
    check()?;
    Ok(None)
}

/// リポジトリの HEAD が指すブランチ名（detached / unborn は表示用文字列）。
pub(crate) fn get_branch_name_for_repo<E: From<git2::Error>>(
    repo: &Repository,
    check: &dyn Fn() -> Result<(), E>,
) -> Result<String, E> {
    check()?;
    let head = repo.head();
    check()?;
    Ok(match head {
        Ok(head) => {
            if head.is_branch() {
                head.shorthand().map_err(E::from)?.to_string()
            } else {
                let oid = head.target().map(|o| o.to_string());
                match oid {
                    Some(h) => format!("({})", &h[..7.min(h.len())]),
                    None => "HEAD".to_string(),
                }
            }
        }
        Err(e) if e.code() == ErrorCode::UnbornBranch => "(no commits)".to_string(),
        Err(error) => return Err(E::from(error)),
    })
}
