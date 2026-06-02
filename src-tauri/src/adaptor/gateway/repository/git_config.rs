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
}

#[cfg(test)]
mod git_config_gateway_tests {
    use super::*;
    use crate::git::test_helpers::*;

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
