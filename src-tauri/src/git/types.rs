use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
}

#[derive(Serialize)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
}

#[derive(Serialize, Debug, Clone)]
pub struct WorktreeEntry {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_locked: bool,
    pub dirty_count: u32,
    pub base_branch: Option<String>,
}

#[derive(Serialize)]
pub struct BranchCard {
    pub name: String,
    pub is_default: bool,
    pub worktree_path: Option<String>,
    pub dirty_count: usize,
    pub is_merged: bool,
    pub has_pr: bool,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub is_remote_only: bool,
    pub has_upstream: bool,
    pub remote_name: Option<String>,
}
