use crate::adaptor::gateway::shared::git_operation;
pub fn get_origin_url(
    repo_path: &str,
) -> Result<Option<String>, crate::adaptor::gateway::shared::git_operation::GitOperationError> {
    let Some(repo) =
        git_operation::optional(git_operation::run(|| git2::Repository::open(repo_path)))?
    else {
        return Ok(None);
    };
    let Some(remote) = git_operation::optional(git_operation::run(|| repo.find_remote("origin")))?
    else {
        return Ok(None);
    };
    Ok(Some(remote.url()?.to_string()))
}

pub(super) fn is_github(url: &str) -> bool {
    url.contains("github.com")
}

pub fn is_github_repository(
    repo_path: &str,
) -> Result<bool, crate::adaptor::gateway::shared::git_operation::GitOperationError> {
    Ok(get_origin_url(repo_path)?.is_some_and(|url| is_github(&url)))
}

#[cfg(test)]
#[path = "discovery_test.rs"]
mod discovery_tests;
