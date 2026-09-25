use crate::domain::operation_context::OperationStopped;

#[derive(Debug)]
pub(crate) enum GitOperationError {
    Git(git2::Error),
    Stopped(OperationStopped),
}
impl GitOperationError {
    pub fn code(&self) -> git2::ErrorCode {
        match self {
            Self::Git(error) => error.code(),
            Self::Stopped(_) => git2::ErrorCode::User,
        }
    }
}
impl std::fmt::Display for GitOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git(error) => error.fmt(f),
            Self::Stopped(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for GitOperationError {}
impl From<git2::Error> for GitOperationError {
    fn from(error: git2::Error) -> Self {
        Self::Git(error)
    }
}

pub(crate) fn run<T>(
    operation: impl FnOnce() -> Result<T, git2::Error>,
) -> Result<T, GitOperationError> {
    crate::other::operation_context::check().map_err(GitOperationError::Stopped)?;
    let result = operation();
    crate::other::operation_context::check().map_err(GitOperationError::Stopped)?;
    result.map_err(GitOperationError::Git)
}

impl From<GitOperationError> for crate::domain::code::CodeError {
    fn from(error: GitOperationError) -> Self {
        match error {
            GitOperationError::Git(error) => error.into(),
            GitOperationError::Stopped(error) => error.into(),
        }
    }
}
impl From<GitOperationError> for crate::domain::repository::RepositoryError {
    fn from(error: GitOperationError) -> Self {
        match error {
            GitOperationError::Git(error) => error.into(),
            GitOperationError::Stopped(error) => error.into(),
        }
    }
}

pub(crate) fn checkout() -> git2::build::CheckoutBuilder<'static> {
    let context = crate::other::operation_context::current();
    let mut options = git2::build::CheckoutBuilder::new();
    options.notify_on(git2::CheckoutNotificationType::all());
    options.notify(move |_, _, _, _, _| context.check(std::time::Instant::now()).is_ok());
    options
}

#[cfg(test)]
#[path = "git_operation_test.rs"]
mod git_operation_tests;

pub(crate) fn detect_default_branch(
    repo: &git2::Repository,
) -> Result<Option<String>, OperationStopped> {
    crate::infrastructure::git::helpers::detect_default_branch(
        repo,
        &crate::other::operation_context::check,
    )
}
pub(crate) fn get_branch_name_for_repo(
    repo: &git2::Repository,
) -> Result<String, OperationStopped> {
    crate::infrastructure::git::helpers::get_branch_name_for_repo(
        repo,
        &crate::other::operation_context::check,
    )
}
pub(crate) fn optional<T>(
    result: Result<T, GitOperationError>,
) -> Result<Option<T>, OperationStopped> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(GitOperationError::Git(_)) => Ok(None),
        Err(GitOperationError::Stopped(error)) => Err(error),
    }
}
