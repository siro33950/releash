/// 作業ツリー上の 1 ファイルの index / worktree ステータス。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    /// conflict しているファイルでは、その種類。conflict していないファイルでは `None`。
    pub conflict: Option<ConflictKind>,
}

impl FileStatus {
    /// stage 済みの変更の種類を決める。
    ///
    /// - index の状態が `new` なら `Added`、`deleted` なら `Deleted`、`modified` と `renamed` なら `Modified` である。
    /// - index の状態が `none` のファイルと、conflict しているファイルでは `None` である。
    pub fn staged_change(&self) -> Option<ChangeKind> {
        if self.conflict.is_some() {
            return None;
        }
        match self.index_status.as_str() {
            "new" => Some(ChangeKind::Added),
            "deleted" => Some(ChangeKind::Deleted),
            "modified" | "renamed" => Some(ChangeKind::Modified),
            _ => None,
        }
    }

    /// まだ stage していない変更の種類を決める。
    ///
    /// - worktree の状態が `new` なら `Untracked`、`deleted` なら `Deleted`、`modified` なら `Modified` である。
    /// - worktree の状態が `none` と `ignored` のファイルと、conflict しているファイルでは `None` である。
    pub fn unstaged_change(&self) -> Option<ChangeKind> {
        if self.conflict.is_some() {
            return None;
        }
        match self.worktree_status.as_str() {
            "new" => Some(ChangeKind::Untracked),
            "deleted" => Some(ChangeKind::Deleted),
            "modified" => Some(ChangeKind::Modified),
            _ => None,
        }
    }
}

/// 変更の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// 中身を変えた。
    Modified,
    /// 新しく足して stage した。
    Added,
    /// 消した。
    Deleted,
    /// git が追跡していない。
    Untracked,
}

/// conflict の種類。git の status が出す区別と同じである。
///
/// 走査結果を運ぶときは、`both_modified`、`both_added`、`both_deleted`、`added_by_us`、
/// `added_by_them`、`deleted_by_us`、`deleted_by_them` のどれかの文字列になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// 両方の側が変更した。
    BothModified,
    /// 両方の側が追加した。
    BothAdded,
    /// 両方の側が削除した。
    BothDeleted,
    /// こちらの側だけが追加した。
    AddedByUs,
    /// 相手の側だけが追加した。
    AddedByThem,
    /// こちらの側が削除し、相手の側が変更した。
    DeletedByUs,
    /// 相手の側が削除し、こちらの側が変更した。
    DeletedByThem,
}

/// 1 scan サイクル内で同一 repository handle から導出した status 系 read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryStatusScan {
    pub status: Vec<FileStatus>,
    pub dirty_count: usize,
}

#[cfg(test)]
#[path = "file_status_test.rs"]
mod file_status_tests;
