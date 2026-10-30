/// 作業ツリー上の 1 ファイルの index / worktree ステータス。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
}

/// ステージ済み / 未ステージの追加・削除行数の集計。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiffStat {
    pub path: String,
    pub index_additions: u32,
    pub index_deletions: u32,
    pub wt_additions: u32,
    pub wt_deletions: u32,
}

/// 1 scan サイクル内で同一 repository handle から導出した status 系 read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryStatusScan {
    pub status: Vec<FileStatus>,
    pub diff_stats: Vec<FileDiffStat>,
    pub dirty_count: usize,
}
