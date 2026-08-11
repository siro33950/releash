//! git_config 責務の gateway 実装。releash base（global / per-branch）の
//! git config 読み書きを封じ込める。

use super::util::resolve_branch_base;
use crate::domain::repository::{GitConfigRepository, RepositoryError};
use crate::infrastructure::git::client;

pub(crate) fn get_branch_base(
    repo_path: &str,
    branch_name: &str,
) -> Result<Option<String>, RepositoryError> {
    let repo = client::open(repo_path)?;
    let config = repo.config().ok();
    Ok(resolve_branch_base(&repo, config.as_ref(), branch_name))
}

/// `value` が `Some` なら `key` を設定、`None` なら削除する。削除時の
/// `NotFound` は許容し（既に無い状態は成功扱い）、他のエラーは伝播する。
/// `set_branch_base_override` / `set_releash_base` 共通の非自明分岐を 1 箇所に集約する。
fn set_or_remove(
    config: &mut git2::Config,
    key: &str,
    value: Option<&str>,
) -> Result<(), RepositoryError> {
    match value {
        Some(v) => config.set_str(key, v)?,
        None => match config.remove(key) {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

pub(crate) fn set_branch_base_override(
    repo_path: &str,
    branch_name: &str,
    base: Option<&str>,
) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;
    let mut config = repo.config()?;
    let key = format!("branch.{branch_name}.releash-base");
    set_or_remove(&mut config, &key, base)
}

pub(crate) fn get_releash_base(repo_path: &str) -> Result<Option<String>, RepositoryError> {
    let repo = client::open(repo_path)?;
    let base = repo
        .config()
        .ok()
        .and_then(|cfg| cfg.get_string("releash.base").ok());
    Ok(base)
}

pub(crate) fn set_releash_base(repo_path: &str, base: Option<&str>) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;
    let mut config = repo.config()?;
    set_or_remove(&mut config, "releash.base", base)
}

/// `existing_branches` に含まれないブランチの `branch.*.releash-base` エントリを掃除する。
pub(crate) fn prune_stale_branch_bases(
    repo_path: &str,
    existing_branches: &[String],
) -> Result<(), RepositoryError> {
    let repo = client::open(repo_path)?;
    let existing: std::collections::HashSet<&str> =
        existing_branches.iter().map(|s| s.as_str()).collect();
    if let Ok(mut gc_cfg) = repo.config() {
        if let Ok(snap) = gc_cfg.snapshot() {
            let mut to_remove = Vec::new();
            if let Ok(mut entries) = snap.entries(Some("branch.*.releash-base")) {
                while let Some(Ok(entry)) = entries.next() {
                    if let Ok(entry_name) = entry.name() {
                        if let Some(branch_name) = entry_name
                            .strip_prefix("branch.")
                            .and_then(|s| s.strip_suffix(".releash-base"))
                        {
                            if !existing.contains(branch_name) {
                                to_remove.push(entry_name.to_string());
                            }
                        }
                    }
                }
            }
            drop(snap);
            for key in &to_remove {
                let _ = gc_cfg.remove(key);
            }
        }
    }
    Ok(())
}

/// `path_hint` からリポジトリを discover する。`path_hint` が存在しないファイル
/// （削除済みファイル等）の場合は親ディレクトリから discover する。
fn discover_repo(path: &std::path::Path) -> Result<git2::Repository, RepositoryError> {
    match client::discover(path) {
        Ok(repo) => Ok(repo),
        Err(_) if !path.exists() => {
            if let Some(parent) = path.parent() {
                Ok(client::discover(parent)?)
            } else {
                Ok(client::discover(path)?)
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// 現在ブランチのベースブランチ名を解決する（per-branch override → global → default）。
/// detached HEAD / unborn / 解決不可は `None`。ref 存在検証・merge-base は行わない。
pub(crate) fn resolve_current_base_branch(
    path_hint: &str,
) -> Result<Option<String>, RepositoryError> {
    let repo = discover_repo(std::path::Path::new(path_hint))?;
    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !head.is_branch() {
        return Ok(None);
    }
    let branch_name = head.shorthand()?;
    let config = repo.config().ok();
    Ok(resolve_branch_base(&repo, config.as_ref(), branch_name))
}

/// 開いた repo で base 名 → local（`refs/heads/<name>`）→ remote
/// （`refs/remotes/origin/<name>`）の順に ref を解決し、base コミットの OID を返す。
/// いずれの ref も実在しない場合は `None`。
fn resolve_base_ref_oid(repo: &git2::Repository, base_name: &str) -> Option<git2::Oid> {
    let peel = |reference: &str| {
        repo.revparse_single(reference)
            .ok()
            .and_then(|obj| obj.peel_to_commit().ok())
            .map(|commit| commit.id())
    };
    peel(&format!("refs/heads/{base_name}"))
        .or_else(|| peel(&format!("refs/remotes/origin/{base_name}")))
}

pub(crate) fn resolve_base_commit_oid(
    path_hint: &str,
    base_name: &str,
) -> Result<Option<String>, RepositoryError> {
    let repo = discover_repo(std::path::Path::new(path_hint))?;
    Ok(resolve_base_ref_oid(&repo, base_name).map(|oid| oid.to_string()))
}

/// `GitConfigRepository` の git2 実装。
pub struct GitConfigGateway;

impl GitConfigRepository for GitConfigGateway {
    fn get_releash_base(&self, repo_path: &str) -> Result<Option<String>, RepositoryError> {
        get_releash_base(repo_path)
    }
    fn set_releash_base(&self, repo_path: &str, base: Option<&str>) -> Result<(), RepositoryError> {
        set_releash_base(repo_path, base)
    }
    fn get_branch_base(
        &self,
        repo_path: &str,
        branch_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        get_branch_base(repo_path, branch_name)
    }
    fn set_branch_base_override(
        &self,
        repo_path: &str,
        branch_name: &str,
        base: Option<&str>,
    ) -> Result<(), RepositoryError> {
        set_branch_base_override(repo_path, branch_name, base)
    }
    fn prune_stale_branch_bases(
        &self,
        repo_path: &str,
        existing_branches: &[String],
    ) -> Result<(), RepositoryError> {
        prune_stale_branch_bases(repo_path, existing_branches)
    }
    fn resolve_current_base_branch(
        &self,
        path_hint: &str,
    ) -> Result<Option<String>, RepositoryError> {
        resolve_current_base_branch(path_hint)
    }
    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        resolve_base_commit_oid(path_hint, base_name)
    }
}

#[cfg(test)]
mod git_config_gateway_tests {
    use super::*;
    use crate::test_support::git::*;

    #[test]
    fn test_ベース解決_per_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    fn test_ベース解決_releash_baseフォールバック() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_releash_base(repo_path, Some("develop")).unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    fn test_ベース解決_既定ブランチフォールバック() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat");
        assert!(result.is_some());
    }

    #[test]
    fn test_branch_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        let base = get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let base = get_branch_base(repo_path, "feat").unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_branch_base_override(repo_path, "feat", None).unwrap();
        let base = get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());
        assert_ne!(base, Some("develop".to_string()));
    }

    #[test]
    fn test_releash_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap();

        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);

        set_releash_base(repo_path, Some("develop")).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_releash_base(repo_path, None).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);
    }

    fn checkout_feature_branch(repo: &git2::Repository) {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }

    #[test]
    fn test_現在ブランチbase解決_override優先() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        checkout_feature_branch(&repo);
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();

        let result = resolve_current_base_branch(repo_path).unwrap();
        assert_eq!(result, Some(default_branch));
    }

    #[test]
    fn test_現在ブランチbase解決_detached_none() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);
        repo.set_head_detached(oid).unwrap();

        let result = resolve_current_base_branch(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_gc_現存しないブランチのbaseを掃除() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "alive", Some("main")).unwrap();
        set_branch_base_override(repo_path, "stale", Some("main")).unwrap();

        // "alive" のみ現存ブランチとして渡すと "stale" の base が掃除される
        prune_stale_branch_bases(repo_path, &["alive".to_string()]).unwrap();

        let config = repo.config().unwrap();
        assert!(config.get_string("branch.alive.releash-base").is_ok());
        assert!(config.get_string("branch.stale.releash-base").is_err());
    }
}
