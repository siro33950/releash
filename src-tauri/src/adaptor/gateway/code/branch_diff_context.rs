use std::sync::Arc;

use crate::usecase::agent_session::context::{
    BranchDiffContextChangedFile, BranchDiffContextPort, BranchDiffContextStats,
    BranchDiffContextSummary,
};
use crate::usecase::code_usecase::CodeUsecase;

pub(crate) struct CodeBranchDiffContextGateway {
    code_usecase: Arc<CodeUsecase>,
}

impl CodeBranchDiffContextGateway {
    pub(crate) fn new(code_usecase: Arc<CodeUsecase>) -> Self {
        Self { code_usecase }
    }
}

impl BranchDiffContextPort for CodeBranchDiffContextGateway {
    fn get_branch_diff_context(
        &self,
        worktree_path: &str,
    ) -> Result<BranchDiffContextSummary, String> {
        let summary = self
            .code_usecase
            .get_branch_diff_summary(worktree_path, None)
            .map_err(|error| error.to_string())?;
        Ok(BranchDiffContextSummary {
            base_branch: summary.base_branch,
            changed_files: summary
                .changed_files
                .into_iter()
                .map(|file| BranchDiffContextChangedFile {
                    path: file.path,
                    status: file.status,
                    stats: BranchDiffContextStats {
                        additions: file.stats.additions,
                        deletions: file.stats.deletions,
                    },
                })
                .collect(),
        })
    }
}
