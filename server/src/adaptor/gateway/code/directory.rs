use std::path::{Component, Path};

use crate::adaptor::gateway::shared::git_operation;
use crate::domain::code::CodeError;
use crate::usecase::code_error::CodeUsecaseError;
use crate::usecase::directory_query::{DirectoryEntryDto, DirectoryQuery};

pub struct DirectoryQueryGateway;

impl DirectoryQuery for DirectoryQueryGateway {
    fn file_target(&self, worktree: &str, file: &str) -> Result<String, CodeUsecaseError> {
        let relative = Path::new(file);
        if relative.components().any(|part| {
            !matches!(part, Component::Normal(_) | Component::CurDir) || part.as_os_str() == ".git"
        }) {
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
        statuses: &[crate::usecase::repository_dto::FileStatusDto],
    ) -> Result<Vec<DirectoryEntryDto>, CodeUsecaseError> {
        let relative = Path::new(directory);
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
        {
            return Err(CodeError::Rule("invalid directory path".into()).into());
        }
        let root = std::fs::canonicalize(worktree).map_err(CodeError::from)?;
        let target = std::fs::canonicalize(root.join(relative)).map_err(CodeError::from)?;
        if !target.starts_with(&root)
            || relative.components().any(|part| part.as_os_str() == ".git")
        {
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
            if name == ".git" {
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
            let status = crate::domain::code::services::directory::aggregate_status(
                statuses
                    .iter()
                    .filter(|entry| {
                        entry.path == path
                            || (kind.is_dir() && entry.path.starts_with(&format!("{path}/")))
                    })
                    .flat_map(|entry| {
                        [entry.index_status.as_str(), entry.worktree_status.as_str()]
                    }),
            );
            entries.push(DirectoryEntryDto {
                path,
                name,
                directory: kind.is_dir(),
                ignored,
                status: status.into(),
            });
        }
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(entries)
    }
}
