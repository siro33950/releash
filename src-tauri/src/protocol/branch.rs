use serde::{Deserialize, Serialize};

use crate::git::types::WorktreeBranch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCardMsg {
    pub name: String,
    pub is_default: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    #[serde(default)]
    pub has_pr: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub behind: usize,
    #[serde(default)]
    pub has_upstream: bool,
    #[serde(default)]
    pub base_ahead: usize,
}

impl From<WorktreeBranch> for BranchCardMsg {
    fn from(b: WorktreeBranch) -> Self {
        Self {
            name: b.name,
            is_default: b.is_default,
            worktree_path: b.worktree_path,
            dirty_count: b.dirty_count,
            is_merged: b.is_merged,
            has_pr: b.has_pr,
            pr_number: b.pr_number,
            pr_url: b.pr_url,
            ahead: b.ahead,
            behind: b.behind,
            has_upstream: b.has_upstream,
            base_ahead: b.base_ahead,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListSync {
    pub branches: Vec<BranchCardMsg>,
}
