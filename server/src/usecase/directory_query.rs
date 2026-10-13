use super::code_error::CodeUsecaseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryDto {
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub ignored: bool,
    pub status: crate::domain::code::services::directory::GitStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub ignored: bool,
}

pub trait DirectoryQuery: Send + Sync {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError>;
    fn entries(
        &self,
        worktree: &str,
        directory: &str,
    ) -> Result<Vec<DirectoryEntry>, CodeUsecaseError>;
}
