use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCardMsg {
    pub name: String,
    pub is_default: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchListSync {
    pub branches: Vec<BranchCardMsg>,
}
