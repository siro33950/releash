use super::code_error::CodeUsecaseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntryDto {
    pub path: String,
    pub name: String,
    pub directory: bool,
    pub ignored: bool,
    pub status: String,
}

pub trait DirectoryQuery: Send + Sync {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError>;
    fn entries(
        &self,
        worktree: &str,
        directory: &str,
        statuses: &[super::repository_dto::FileStatusDto],
    ) -> Result<Vec<DirectoryEntryDto>, CodeUsecaseError>;
}
