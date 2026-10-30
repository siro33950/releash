//! リポジトリパス解決ユーティリティの gateway 実装。

use crate::domain::git::{GitError, RepoLocator};

pub(crate) fn get_cwd() -> Result<String, GitError> {
    std::env::current_dir()?
        .to_str()
        .ok_or_else(|| GitError::rule("invalid path encoding"))
        .map(|s| s.to_string())
}

/// `RepoLocator` の実装。
pub struct RepoLocatorGateway;

impl RepoLocator for RepoLocatorGateway {
    fn cwd(&self) -> Result<String, GitError> {
        get_cwd()
    }
}
