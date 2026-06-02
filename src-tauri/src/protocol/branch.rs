use serde::{Deserialize, Serialize};

use crate::usecase::repository_dto::BranchCardDto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCardMsg {
    pub name: String,
    #[serde(default)]
    pub is_main_worktree: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    #[serde(default)]
    pub ahead: usize,
    #[serde(default)]
    pub behind: usize,
    #[serde(default)]
    pub has_upstream: bool,
    #[serde(default)]
    pub base_ahead: usize,
}

impl From<BranchCardDto> for BranchCardMsg {
    fn from(b: BranchCardDto) -> Self {
        Self {
            name: b.name,
            is_main_worktree: b.is_main_worktree,
            worktree_path: b.worktree_path,
            dirty_count: b.dirty_count,
            is_merged: b.is_merged,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfoRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfoResponse {
    pub branch: String,
}
