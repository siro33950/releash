use crate::domain::code::services::directory::worktree_path_is_visible;
use std::path::Path;

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::code::CodeError;
use crate::usecase::code_error::CodeUsecaseError;
use crate::usecase::directory_query::{DirectoryEntry, DirectoryQuery};

pub struct DirectoryQueryGateway;

impl DirectoryQuery for DirectoryQueryGateway {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError> {
        let relative = Path::new(file);
        if !worktree_path_is_visible(file) {
            return Err(CodeError::Rule("invalid file path".into()).into());
        }
        let root = std::fs::canonicalize(worktree).map_err(CodeError::from)?;
        let target = root.join(relative);
        let ancestor = target
            .ancestors()
            .find(|path| path.exists())
            .ok_or_else(|| CodeError::Rule("missing file ancestor".into()))?;
        if !std::fs::canonicalize(ancestor)
            .map_err(CodeError::from)?
            .starts_with(&root)
        {
            return Err(CodeError::Rule("file is outside worktree".into()).into());
        }
        target
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| CodeError::Rule("invalid path encoding".into()).into())
    }

    fn entries(
        &self,
        worktree: &str,
        directory: &str,
    ) -> Result<Vec<DirectoryEntry>, CodeUsecaseError> {
        let relative = Path::new(directory);
        if !worktree_path_is_visible(directory) {
            return Err(CodeError::Rule("invalid directory path".into()).into());
        }
        let root = std::fs::canonicalize(worktree).map_err(CodeError::from)?;
        let target = std::fs::canonicalize(root.join(relative)).map_err(CodeError::from)?;
        if !target.starts_with(&root) {
            return Err(CodeError::Rule("directory is outside worktree".into()).into());
        }
        let repo = git_operation::run(|| git2::Repository::open(&root)).map_err(CodeError::from)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(target).map_err(CodeError::from)? {
            let entry = entry.map_err(CodeError::from)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| CodeError::Rule("invalid path encoding".into()))?;
            if !worktree_path_is_visible(&name) {
                continue;
            }
            let kind = entry.file_type().map_err(CodeError::from)?;
            let path = entry
                .path()
                .strip_prefix(&root)
                .map_err(CodeError::from)?
                .to_str()
                .ok_or_else(|| CodeError::Rule("invalid path encoding".into()))?
                .to_owned();
            let ignored = git_operation::run(|| repo.status_should_ignore(Path::new(&path)))
                .map_err(CodeError::from)?;
            entries.push(DirectoryEntry {
                path,
                name,
                directory: kind.is_dir(),
                ignored,
            });
        }
        Ok(entries)
    }
}
