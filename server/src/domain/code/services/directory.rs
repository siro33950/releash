use std::path::{Component, Path};

pub fn worktree_path_is_visible(path: &str) -> bool {
    Path::new(path).components().all(|part| {
        matches!(part, Component::Normal(_) | Component::CurDir) && part.as_os_str() != ".git"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitStatus {
    None,
    New,
    Modified,
    Renamed,
    Deleted,
    Conflicted,
}

impl GitStatus {
    pub fn parse(value: &str) -> Result<Self, crate::domain::code::CodeError> {
        match value {
            "none" => Ok(Self::None),
            "new" => Ok(Self::New),
            "modified" => Ok(Self::Modified),
            "renamed" => Ok(Self::Renamed),
            "deleted" => Ok(Self::Deleted),
            "conflicted" => Ok(Self::Conflicted),
            _ => Err(crate::domain::code::CodeError::Rule(format!(
                "invalid git status: {value}"
            ))),
        }
    }
}

pub fn aggregate_status(statuses: impl Iterator<Item = GitStatus>) -> GitStatus {
    statuses
        .map(|status| {
            if status == GitStatus::Renamed {
                GitStatus::Modified
            } else {
                status
            }
        })
        .max()
        .unwrap_or(GitStatus::None)
}

pub fn entry_status<'a>(
    path: &str,
    directory: bool,
    statuses: impl Iterator<Item = (&'a str, GitStatus, GitStatus)>,
) -> GitStatus {
    let prefix = format!("{path}/");
    aggregate_status(
        statuses
            .filter(|(file, _, _)| *file == path || (directory && file.starts_with(&prefix)))
            .flat_map(|(_, index, working)| [index, working]),
    )
}

pub fn entry_order(
    a_directory: bool,
    a_name: &str,
    b_directory: bool,
    b_name: &str,
) -> std::cmp::Ordering {
    b_directory
        .cmp(&a_directory)
        .then_with(|| a_name.cmp(b_name))
}

#[cfg(test)]
#[path = "directory_test.rs"]
mod directory_tests;
