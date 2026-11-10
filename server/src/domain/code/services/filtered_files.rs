//! worktree の項目から、絞り込んだファイル一覧を組み立てる。

use std::collections::{HashMap, HashSet};

use crate::domain::code::services::directory::{aggregate_status, entry_order, GitStatus};
use crate::domain::code::value_objects::file_list::{
    FileFilter, FileNode, FilteredFiles, WalkStep, WorktreeEntry,
};

/// 検索の入力があるときに、ファイル一覧に残すファイルの上限。
pub const MATCHED_FILE_LIMIT: usize = 512;

/// worktree の項目を一つずつ受け取り、絞り込んだファイル一覧を組み立てる。
///
/// 項目は、ディレクトリをその中の項目より先に受け取る。
pub struct FilteredFilesCollector {
    filter: FileFilter,
    /// path ごとの git の状態。ディレクトリは、中の変更を `entry_status` と同じ規則で集約した状態を持つ。
    /// 中の変更を集約した状態の key は、末尾に `/` を付けた path である。
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
        let mut by_path: HashMap<String, GitStatus> = HashMap::new();
        for (path, index, working) in statuses {
            let status = aggregate_status([index, working].into_iter());
            let ancestors = path.match_indices('/').map(|(end, _)| &path[..=end]);
            for key in ancestors.chain([path]) {
                let merged = by_path.entry(key.to_owned()).or_insert(GitStatus::None);
                *merged = status.max(*merged);
            }
        }
        Self {
            filter,
            statuses: by_path,
            directories: HashMap::new(),
            kept: Vec::new(),
            kept_files: 0,
            truncated: false,
        }
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
        let changed = self.status(&entry) != GitStatus::None;
        let named = self
            .filter
            .name()
            .is_none_or(|query| query.matches(&entry.name));
        if entry.directory {
            if self.filter.changed() {
                if !changed {
                    return WalkStep::SkipChildren;
                }
            } else if named {
                self.kept.push(entry.clone());
            }
            self.directories.insert(entry.path.clone(), entry);
            return WalkStep::Continue;
        }
        if !named || (self.filter.changed() && !changed) {
            return WalkStep::Continue;
        }
        if self.filter.name().is_some() && self.kept_files == MATCHED_FILE_LIMIT {
            self.truncated = true;
            return WalkStep::Stop;
        }
        self.kept_files += 1;
        self.kept.push(entry);
        WalkStep::Continue
    }

    /// 残した項目と、そこへ至るディレクトリを木にする。
    ///
    /// 兄弟の並びは `entry_order` に従う。各項目の git の状態は `entry_status` と同じ規則で決める。
    /// `truncated` は、条件に合うファイルが上限を超えたかである。
    pub fn finish(self) -> FilteredFiles {
        let mut children: HashMap<&str, Vec<&WorktreeEntry>> = HashMap::new();
        let mut placed = HashSet::new();
        for kept in &self.kept {
            let mut entry = kept;
            while placed.insert(entry.path.as_str()) {
                let parent = entry.path.rsplit_once('/').map_or("", |(parent, _)| parent);
                children.entry(parent).or_default().push(entry);
                match self.directories.get(parent) {
                    Some(directory) => entry = directory,
                    None => break,
                }
            }
        }
        FilteredFiles {
            roots: self.nodes("", &mut children),
            truncated: self.truncated,
        }
    }

    fn status(&self, entry: &WorktreeEntry) -> GitStatus {
        let of = |key: &str| self.statuses.get(key).copied().unwrap_or(GitStatus::None);
        let own = of(&entry.path);
        if entry.directory {
            own.max(of(&format!("{}/", entry.path)))
        } else {
            own
        }
    }

    fn nodes(
        &self,
        parent: &str,
        children: &mut HashMap<&str, Vec<&WorktreeEntry>>,
    ) -> Vec<FileNode> {
        let mut entries = children.remove(parent).unwrap_or_default();
        entries.sort_by(|a, b| entry_order(a.directory, &a.name, b.directory, &b.name));
        entries
            .into_iter()
            .map(|entry| FileNode {
                entry: entry.clone(),
                status: self.status(entry),
                children: self.nodes(&entry.path, children),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "filtered_files_test.rs"]
mod filtered_files_tests;
