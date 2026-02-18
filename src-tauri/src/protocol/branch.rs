use serde::{Deserialize, Serialize};

use crate::git::types::BranchCard;

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
    pub is_remote_only: bool,
    #[serde(default)]
    pub has_upstream: bool,
}

impl From<BranchCard> for BranchCardMsg {
    fn from(b: BranchCard) -> Self {
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
            is_remote_only: b.is_remote_only,
            has_upstream: b.has_upstream,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListSync {
    pub branches: Vec<BranchCardMsg>,
}
