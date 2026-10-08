//! ベースブランチ名解決（`BranchBaseResolver` port）の gateway 実装。
//!
//! ベースブランチ名の解決ルール（`branch.<name>.releash-base` → `releash.base` →
//! `detect_default_branch()`）と ref 実在検証は `repository` ドメインが所有する。本実装は
//! `code` 側の port を repository の `GitConfigRepository` ポートへ橋渡しするだけで、解決
//! ロジックを再実装しない（単一情報源を repository に保つ）。`RepositoryError` は文字列を
//! 保持したまま `CodeError` へ畳み込み、エラー表現を移行前と等価に保つ。

use std::sync::Arc;

use crate::domain::code::{BranchBaseResolver, CodeError};
use crate::domain::repository::{GitConfigRepository, RepositoryError};

/// `RepositoryError` を `CodeError` へ畳み込む（メッセージ文字列を保持）。
fn to_code_error(e: RepositoryError) -> CodeError {
    CodeError::External(e.to_string())
}

/// `BranchBaseResolver` の repository 委譲実装。
pub struct BranchBaseResolverGateway {
    git_config: Arc<dyn GitConfigRepository>,
}

impl BranchBaseResolverGateway {
    pub fn new(git_config: Arc<dyn GitConfigRepository>) -> Self {
        Self { git_config }
    }
}

impl BranchBaseResolver for BranchBaseResolverGateway {
    fn resolve_base_branch_name(&self, path_hint: &str) -> Result<Option<String>, CodeError> {
        self.git_config
            .resolve_current_base_branch(path_hint)
            .map_err(to_code_error)
    }

    fn resolve_base_commit_oid(
        &self,
        path_hint: &str,
        base_name: &str,
    ) -> Result<Option<String>, CodeError> {
        self.git_config
            .resolve_base_commit_oid(path_hint, base_name)
            .map_err(to_code_error)
    }
}
