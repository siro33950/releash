//! git_config 責務の gateway 実装。releash base（global / per-branch）の
//! git config 読み書きを封じ込める。

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::git::{GitError, GitService};
use crate::infrastructure::git::client;

/// `value` が `Some` なら `key` を設定、`None` なら削除する。削除時の
/// `NotFound` は許容し（既に無い状態は成功扱い）、他のエラーは伝播する。
/// `set_branch_base_override` / `set_releash_base` 共通の非自明分岐を 1 箇所に集約する。
fn set_or_remove(
    config: &mut git2::Config,
    key: &str,
    value: Option<&str>,
) -> Result<(), GitError> {
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
) -> Result<(), GitError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let mut config = git_operation::run(|| repo.config())?;
    let key = format!("branch.{branch_name}.releash-base");
    set_or_remove(&mut config, &key, base)
}

pub fn get_releash_base(repo_path: &str) -> Result<Option<String>, GitError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let base = match git_operation::optional(git_operation::run(|| repo.config()))? {
        Some(cfg) => {
            git_operation::optional(git_operation::run(|| cfg.get_string("releash.base")))?
        }
        None => None,
    };
    Ok(base)
}

pub fn set_releash_base(repo_path: &str, base: Option<&str>) -> Result<(), GitError> {
    let repo = git_operation::run(|| client::open(repo_path))?;
    let mut config = git_operation::run(|| repo.config())?;
    set_or_remove(&mut config, "releash.base", base)
}

/// `path_hint` からリポジトリを discover する。`path_hint` が存在しないファイル
/// （削除済みファイル等）の場合は親ディレクトリから discover する。
pub(super) fn discover_repo(path: &std::path::Path) -> Result<git2::Repository, GitError> {
    match git_operation::run(|| client::discover(path)) {
        Ok(repo) => Ok(repo),
        Err(error @ git_operation::GitOperationError::Stopped(_)) => Err(error.into()),
        Err(_) if !path.exists() => {
            if let Some(parent) = path.ancestors().skip(1).find(|parent| parent.is_dir()) {
                Ok(git_operation::run(|| client::discover(parent))?)
            } else {
                Ok(git_operation::run(|| client::discover(path))?)
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// 開いた repo で base 名 → local（`refs/heads/<name>`）→ remote
/// （`refs/remotes/origin/<name>`）の順に ref を解決し、base コミットの OID を返す。
/// いずれの ref も実在しない場合は `None`。
fn resolve_base_ref_oid(
    repo: &git2::Repository,
    base_name: &str,
) -> Result<Option<git2::Oid>, GitError> {
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
#[cfg(any(test, feature = "test-support"))]
pub fn resolve_effective_base_branch(repo_path: &str) -> Result<Option<String>, GitError> {
    let git = super::git::GitGateway::new(std::sync::Arc::new(
        crate::adaptor::gateway::app_config::AppConfig::new(
            Default::default(),
            std::path::PathBuf::new(),
        ),
    ));
    resolve_effective_base_branch_with_git(repo_path, &git)
}

pub(crate) fn resolve_effective_base_branch_with_git(
    repo_path: &str,
    git: &dyn GitService,
) -> Result<Option<String>, GitError> {
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
    let Some(current_oid) = head.target() else {
        return Ok(None);
    };
    let Some(base_name) = git.resolve_current_base_branch(repo_path)? else {
        return Ok(None);
    };
    let Some(base_oid) = resolve_base_ref_oid(&repo, &base_name)? else {
        return Ok(None);
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
) -> Result<Option<String>, GitError> {
    let repo = discover_repo(std::path::Path::new(path_hint))?;
    Ok(resolve_base_ref_oid(&repo, base_name)?.map(|oid| oid.to_string()))
}
