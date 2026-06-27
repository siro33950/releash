pub(crate) trait BranchDiffContextPort: Send + Sync {
    fn get_branch_diff_context(
        &self,
        worktree_path: &str,
    ) -> Result<BranchDiffContextSummary, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchDiffContextStats {
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchDiffContextChangedFile {
    pub path: String,
    pub status: String,
    pub stats: BranchDiffContextStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchDiffContextSummary {
    pub base_branch: String,
    pub changed_files: Vec<BranchDiffContextChangedFile>,
}
