//! worktree の項目から、絞り込んだファイル一覧を組み立てる。

use std::collections::HashMap;

use crate::domain::code::services::directory::GitStatus;
use crate::domain::code::value_objects::file_list::{
    FileFilter, FilteredFiles, WalkStep, WorktreeEntry,
};

/// 検索の入力があるときに、ファイル一覧に残すファイルの上限。
pub const MATCHED_FILE_LIMIT: usize = 512;

/// worktree の項目を一つずつ受け取り、絞り込んだファイル一覧を組み立てる。
///
/// 項目は、ディレクトリをその中の項目より先に受け取る。
pub struct FilteredFilesCollector {
    filter: FileFilter,
    /// path ごとの git の状態。ディレクトリは、中の変更を `entry_status` と同じ規則で集約した状態を持つ。
    statuses: HashMap<String, GitStatus>,
    /// 受け取ったディレクトリ。残した項目へ至るディレクトリを引くために持つ。
    directories: HashMap<String, WorktreeEntry>,
    /// 残すと決めた項目。
    kept: Vec<WorktreeEntry>,
    /// 残すと決めたファイルの数。
    kept_files: usize,
    /// 条件に合うファイルが上限を超えたか。
    truncated: bool,
}

impl FilteredFilesCollector {
    /// 条件と、path ごとの index と作業ツリーの git の状態から、空の状態を作る。
    pub fn new<'a>(
        filter: FileFilter,
        statuses: impl Iterator<Item = (&'a str, GitStatus, GitStatus)>,
    ) -> Self {
        todo!()
    }

    /// 項目を一つ受け取り、残すかと、列挙をどう続けるかを決める。
    ///
    /// 残すかは次のとおりに決める。
    /// - ファイルは、条件に合えば残す。`Name` は名前が検索の入力に一致すること、`Changed` は git の状態が付いていること、`NameAndChanged` はその両方である。
    /// - ディレクトリは、条件が `Name` で、名前が検索の入力に一致すれば残す。「差分のみ」を含む条件では、ディレクトリそのものは残さない。
    ///
    /// 列挙の続け方は次のとおりに決める。
    /// - 条件が検索の入力を持ち、条件に合うファイルが `MATCHED_FILE_LIMIT` を超えたら、そのファイルを残さずに `Stop` を返す。
    /// - 条件が「差分のみ」を含み、受け取ったディレクトリに git の状態が付いていなければ、`SkipChildren` を返す。
    /// - それ以外は `Continue` を返す。
    pub fn offer(&mut self, entry: WorktreeEntry) -> WalkStep {
        todo!()
    }

    /// 残した項目と、そこへ至るディレクトリを木にする。
    ///
    /// 兄弟の並びは `entry_order` に従う。各項目の git の状態は `entry_status` と同じ規則で決める。
    /// `truncated` は、条件に合うファイルが上限を超えたかである。
    pub fn finish(self) -> FilteredFiles {
        todo!()
    }
}
