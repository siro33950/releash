//! code が使うベースブランチ名とコミット OID を GitService から取得する。

use std::sync::Arc;

use crate::domain::code::{BranchBaseResolver, CodeError};
use crate::domain::git::{GitError, GitService};

/// `GitError` を `CodeError` へ畳み込む（メッセージ文字列を保持）。
fn to_code_error(e: GitError) -> CodeError {
    CodeError::External(e.to_string())
}

/// code のベースブランチ参照を GitService へ接続する。
pub struct BranchBaseResolverGateway {
    git_config: Arc<dyn GitService>,
}

impl BranchBaseResolverGateway {
    pub fn new(git_config: Arc<dyn GitService>) -> Self {
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
