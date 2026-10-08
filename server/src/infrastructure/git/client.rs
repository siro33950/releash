//! git2 リポジトリへのアクセスを提供する薄いラッパー。
//!
//! ドメイン知識や変換ロジックを持たず、`git2::Repository` の生成のみを
//! 担う。gateway 層はこのラッパー経由でリポジトリを開く。

use git2::Repository;
use std::path::Path;

/// 既知のリポジトリパスを開く（`Repository::open` 相当）。
pub fn open(path: impl AsRef<Path>) -> Result<Repository, git2::Error> {
    Repository::open(path)
}

/// 任意のパスから所属リポジトリを探索する（`Repository::discover` 相当）。
pub fn discover(path: impl AsRef<Path>) -> Result<Repository, git2::Error> {
    Repository::discover(path)
}
