/// 作業ツリー上の 1 ファイルの index / worktree ステータス。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

/// 1 scan サイクル内で同一 repository handle から導出した status 系 read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryStatusScan {
    pub status: Vec<FileStatus>,
    pub dirty_count: usize,
}
