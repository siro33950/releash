use super::code_error::CodeUsecaseError;
use crate::domain::code::value_objects::file_list::{WalkStep, WorktreeEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryDto {
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub ignored: bool,
    pub status: crate::domain::code::services::directory::GitStatus,
}

pub trait DirectoryQuery: Send + Sync {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError>;
    fn entries(
        &self,
        worktree: &str,
        directory: &str,
    ) -> Result<Vec<WorktreeEntry>, CodeUsecaseError>;
    /// worktree の項目を上から順に列挙し、一つずつ `visit` に渡す。
    ///
    /// - ディレクトリは、その中の項目より先に渡す。
    /// - `worktree_path_is_visible` が偽になる項目と、その中の項目は渡さない。
    /// - `visit` が `SkipChildren` を返したディレクトリの中は列挙しない。
    /// - `visit` が `Stop` を返したら、列挙を終えて成功を返す。
    fn walk(
        &self,
        worktree: &str,
        visit: &mut dyn FnMut(WorktreeEntry) -> WalkStep,
    ) -> Result<(), CodeUsecaseError>;
}
