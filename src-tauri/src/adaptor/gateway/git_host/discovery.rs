use crate::adaptor::gateway::shared::git_operation;
pub(super) fn get_origin_url(
    repo_path: &str,
) -> Result<Option<String>, crate::domain::operation_context::OperationStopped> {
    let Some(repo) =
        git_operation::optional(git_operation::run(|| git2::Repository::open(repo_path)))?
    else {
        return Ok(None);
    };
    let Some(remote) = git_operation::optional(git_operation::run(|| repo.find_remote("origin")))?
    else {
        return Ok(None);
    };
    Ok(remote.url().ok().map(|s| s.to_string()))
}

pub(super) fn is_github(url: &str) -> bool {
    url.contains("github.com")
}

pub(super) fn is_github_repository(
    repo_path: &str,
) -> Result<bool, crate::domain::operation_context::OperationStopped> {
    Ok(get_origin_url(repo_path)?.is_some_and(|url| is_github(&url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_origin_url_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        assert!(get_origin_url(dir.path().to_str().unwrap())
            .unwrap()
            .is_none());
    }

    #[test]
    fn get_origin_url_with_github_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://github.com/user/repo.git")
            .unwrap();

        let url = get_origin_url(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(url.contains("github.com"));
    }

    #[test]
    fn is_github_accepts_github_urls() {
        assert!(is_github("https://github.com/user/repo.git"));
        assert!(is_github("git@github.com:user/repo.git"));
    }

    #[test]
    fn is_github_rejects_other_hosts() {
        assert!(!is_github("https://gitlab.com/user/repo.git"));
    }

    #[test]
    fn is_github_repository_returns_false_for_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        assert!(!is_github_repository(dir.path().to_str().unwrap()).unwrap());
    }

    #[test]
    fn is_github_repository_returns_true_for_github_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:user/repo.git")
            .unwrap();

        assert!(is_github_repository(dir.path().to_str().unwrap()).unwrap());
    }
}

#[cfg(test)]
#[path = "discovery_test.rs"]
mod discovery_tests;
