//! ファイル一覧の絞り込みに使う値オブジェクト。

use crate::domain::code::services::directory::GitStatus;
use crate::domain::code::CodeError;

/// worktree の中の項目一つ。`path` は worktree からの相対 path である。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: String,
    pub name: String,
    pub directory: bool,
    /// `.gitignore` で除外されているか。
    pub ignored: bool,
}

/// ファイル一覧の検索の入力。1 文字以上である。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameQuery(String);

impl NameQuery {
    /// 入力から検索の入力を作る。空の入力は検索の入力ではないので失敗する。
    pub fn new(input: &str) -> Result<Self, CodeError> {
        if input.is_empty() {
            return Err(CodeError::Rule("empty file name query".into()));
        }
        Ok(Self(input.to_owned()))
    }

    /// 名前の中に、入力の文字がその順に現れるかを決める。
    /// 文字の間に別の文字があってもよい。大文字と小文字は区別しない。
    pub fn matches(&self, name: &str) -> bool {
        let mut name = name.chars().flat_map(char::to_lowercase);
        self.0
            .chars()
            .flat_map(char::to_lowercase)
            .all(|wanted| name.any(|found| found == wanted))
    }

    /// 利用者が入れたままの入力を返す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// ファイル一覧を絞り込む条件。検索の入力と「差分のみ」の少なくとも一方である。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileFilter {
    /// 名前が検索の入力に一致する項目を残す。
    Name(NameQuery),
    /// git の状態が付いているファイルを残す。
    Changed,
    /// 名前が検索の入力に一致し、git の状態が付いているファイルを残す。
    NameAndChanged(NameQuery),
}

impl FileFilter {
    /// 検索の入力と「差分のみ」の指定から条件を作る。どちらも無い指定は絞り込みではないので失敗する。
    pub fn new(name: Option<NameQuery>, changed: bool) -> Result<Self, CodeError> {
        match (name, changed) {
            (Some(name), false) => Ok(Self::Name(name)),
            (None, true) => Ok(Self::Changed),
            (Some(name), true) => Ok(Self::NameAndChanged(name)),
            (None, false) => Err(CodeError::Rule("empty file filter".into())),
        }
    }

    /// 条件が検索の入力を持つなら、それを返す。
    pub fn name(&self) -> Option<&NameQuery> {
        match self {
            Self::Name(name) | Self::NameAndChanged(name) => Some(name),
            Self::Changed => None,
        }
    }

    /// 条件が「差分のみ」を含むかを返す。
    pub fn changed(&self) -> bool {
        !matches!(self, Self::Name(_))
    }
}

/// 項目を一つ受け取ったあと、worktree の列挙をどう続けるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkStep {
    /// 列挙を続ける。受け取った項目がディレクトリなら、その中も列挙する。
    Continue,
    /// 受け取ったディレクトリの中を列挙せず、次の項目へ進む。
    SkipChildren,
    /// 列挙を終える。
    Stop,
}

/// 絞り込みで残った項目一つと、その下に残った項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNode {
    pub entry: WorktreeEntry,
    pub status: GitStatus,
    /// 直下に残った項目。並びは `entry_order` に従う。ファイルでは空である。
    pub children: Vec<FileNode>,
}

/// 絞り込んだファイル一覧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredFiles {
    /// worktree の直下に残った項目。並びは `entry_order` に従う。
    pub roots: Vec<FileNode>,
    /// 条件に合うファイルが上限を超え、結果が一致の一部だけであるか。
    pub truncated: bool,
}

#[cfg(test)]
#[path = "file_list_test.rs"]
mod file_list_tests;
