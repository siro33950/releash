//! リポジトリパス解決ユーティリティの gateway 実装。

use git2::Repository;

use crate::domain::repository::{RepoLocator, RepositoryError};
use crate::infrastructure::git::client;
use crate::infrastructure::git::helpers::detect_default_branch;

/// ベースブランチ名をフォールバックチェーンで解決する（repository gateway の業務ルール）。
/// `branch.<name>.releash-base` → `releash.base` → `detect_default_branch()`。
///
/// 複数情報源を合成する解決順序は infrastructure の責務外のため、infrastructure の
/// プリミティブ（`detect_default_branch` と raw な config 読み）を gateway 側で組み立てる。
pub(crate) fn resolve_branch_base(
    repo: &Repository,
    config: Option<&git2::Config>,
    branch_name: &str,
) -> Option<String> {
    if let Some(cfg) = config {
        if let Ok(base) = cfg.get_string(&format!("branch.{branch_name}.releash-base")) {
            return Some(base);
        }
        if let Ok(base) = cfg.get_string("releash.base") {
            return Some(base);
        }
    }
    detect_default_branch(repo)
}

pub(crate) fn get_cwd() -> Result<String, RepositoryError> {
    std::env::current_dir()?
        .to_str()
        .ok_or_else(|| RepositoryError::rule("invalid path encoding"))
        .map(|s| s.to_string())
}

pub(crate) fn get_repo_git_dir(file_path: &str) -> Result<String, RepositoryError> {
    let repo = client::discover(file_path)?;

    repo.path()
        .to_str()
        .ok_or_else(|| RepositoryError::rule("invalid path encoding"))
        .map(|s| s.to_string())
}

/// `RepoLocator` の実装。
pub struct RepoLocatorGateway;

impl RepoLocator for RepoLocatorGateway {
    fn cwd(&self) -> Result<String, RepositoryError> {
        get_cwd()
    }
    fn git_dir(&self, file_path: &str) -> Result<String, RepositoryError> {
        get_repo_git_dir(file_path)
    }
}
