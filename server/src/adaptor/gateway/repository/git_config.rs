//! git_config 責務の gateway 実装。releash base（global / per-branch）の
//! git config 読み書きを封じ込める。

use super::util::resolve_branch_base;
use crate::adaptor::gateway::shared::git_operation;
use crate::domain::repository::{GitConfigRepository, RepositoryError};
use crate::infrastructure::git::client;

pub fn get_branch_base(
    repo_path: &str,
    branch_name: &str,
) -> Result<Option<String>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let config = git_operation::optional(git_operation::run(|| repo.config()))?;
    Ok(resolve_branch_base(&repo, config.as_ref(), branch_name)?)
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
        Some(v) => git_operation::run(|| config.set_str(key, v))?,
        None => match git_operation::run(|| config.remove(key)) {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

pub fn set_branch_base_override(
    repo_path: &str,
    branch_name: &str,
    base: Option<&str>,
) -> Result<(), RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let mut config = git_operation::run(|| repo.config())?;
    let key = format!("branch.{branch_name}.releash-base");
    set_or_remove(&mut config, &key, base)
}

pub fn get_releash_base(repo_path: &str) -> Result<Option<String>, RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let base = match git_operation::optional(git_operation::run(|| repo.config()))? {
        Some(cfg) => {
            git_operation::optional(git_operation::run(|| cfg.get_string("releash.base")))?
        }
        None => None,
    };
    Ok(base)
}

pub fn set_releash_base(repo_path: &str, base: Option<&str>) -> Result<(), RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let mut config = git_operation::run(|| repo.config())?;
    set_or_remove(&mut config, "releash.base", base)
}

/// `existing_branches` に含まれないブランチの `branch.*.releash-base` エントリを掃除する。
pub fn prune_stale_branch_bases(
    repo_path: &str,
    existing_branches: &[String],
) -> Result<(), RepositoryError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let existing: std::collections::HashSet<&str> =
        existing_branches.iter().map(|s| s.as_str()).collect();
    if let Some(mut gc_cfg) = git_operation::optional(git_operation::run(|| repo.config()))? {
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
                git_operation::optional(git_operation::run(|| gc_cfg.remove(key)))?;
            }
        }
    }
    Ok(())
}

/// `path_hint` からリポジトリを discover する。`path_hint` が存在しないファイル
/// （削除済みファイル等）の場合は親ディレクトリから discover する。
fn discover_repo(path: &std::path::Path) -> Result<git2::Repository, RepositoryError> {
    match git_operation::run(|| client::discover(path)) {
        Ok(repo) => Ok(repo),
        Err(error @ git_operation::GitOperationError::Stopped(_)) => Err(error.into()),
        Err(_) if !path.exists() => {
            if let Some(parent) = path.parent() {
                Ok(git_operation::run(|| client::discover(parent))?)
            } else {
                Ok(git_operation::run(|| client::discover(path))?)
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// 現在ブランチのベースブランチ名を解決する（per-branch override → global → default）。
/// detached HEAD / unborn / 解決不可は `None`。ref 存在検証・merge-base は行わない。
pub fn resolve_current_base_branch(path_hint: &str) -> Result<Option<String>, RepositoryError> {
    let repo = discover_repo(std::path::Path::new(path_hint))?;
    let head = match git_operation::run(|| repo.head()) {
        Ok(h) => h,
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !head.is_branch() {
        return Ok(None);
    }
    let branch_name = head.shorthand()?;
    let config = git_operation::optional(git_operation::run(|| repo.config()))?;
    Ok(resolve_branch_base(&repo, config.as_ref(), branch_name)?)
}

/// 開いた repo で base 名 → local（`refs/heads/<name>`）→ remote
/// （`refs/remotes/origin/<name>`）の順に ref を解決し、base コミットの OID を返す。
/// いずれの ref も実在しない場合は `None`。
fn resolve_base_ref_oid(
    repo: &git2::Repository,
    base_name: &str,
) -> Result<Option<git2::Oid>, RepositoryError> {
    for reference in [
        format!("refs/heads/{base_name}"),
        format!("refs/remotes/origin/{base_name}"),
    ] {
        let Some(object) =
            git_operation::optional(git_operation::run(|| repo.revparse_single(&reference)))?
        else {
            continue;
        };
        if let Some(commit) =
            git_operation::optional(git_operation::run(|| object.peel_to_commit()))?
        {
            return Ok(Some(commit.id()));
        }
    }
    Ok(None)
}

/// Provider TUI の `RELEASH_BASE_BRANCH` に渡す現在ブランチの実効 base 名を返す。
/// detached / unborn / ref 不在 / merge-base 不成立は `None`。
pub fn resolve_effective_base_branch(repo_path: &str) -> Result<Option<String>, RepositoryError> {
    let Some(repo) = git_operation::optional(git_operation::run(|| client::open(repo_path)))?
    else {
        return Ok(None);
    };
    let Some(head) = git_operation::optional(match git_operation::run(|| repo.head()) {
        Err(error) if error.code() == git2::ErrorCode::UnbornBranch => return Ok(None),
        result => result,
    })?
    else {
        return Ok(None);
    };
    if !head.is_branch() {
        return Ok(None);
    }
    let current_oid = match head.target() {
        Some(oid) => oid,
        None => return Ok(None),
    };
    let branch_name = head.shorthand()?.to_string();
    let config = git_operation::optional(git_operation::run(|| repo.config()))?;
    let base_name = match resolve_branch_base(&repo, config.as_ref(), &branch_name)? {
        Some(base_name) => base_name,
        None => return Ok(None),
    };
    let base_oid = match resolve_base_ref_oid(&repo, &base_name)? {
        Some(base_oid) => base_oid,
        None => return Ok(None),
    };
    if git_operation::optional(git_operation::run(|| {
        repo.merge_base(current_oid, base_oid)
    }))?
    .is_none()
    {
        return Ok(None);
    }
    Ok(Some(base_name))
}

pub fn resolve_base_commit_oid(
    path_hint: &str,
    base_name: &str,
) -> Result<Option<String>, RepositoryError> {
    let repo = discover_repo(std::path::Path::new(path_hint))?;
    Ok(resolve_base_ref_oid(&repo, base_name)?.map(|oid| oid.to_string()))
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
