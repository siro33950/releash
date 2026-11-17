use crate::domain::git::Branch;
use crate::domain::git_host::PullRequestCreationError;

pub fn issue_branch_name(number: u64) -> String {
    format!("feat/issues/{number}")
}

/// `head` のブランチから PR を作れるかを決める。
///
/// 次の順に確かめ、最初に当たった理由で拒む。
/// 1. origin が GitHub のリポジトリでない（`origin_url` が無いか、`GitHubRepository::from_origin_url` が読み取れない）
///    ときは、`NotGitHubRepository`。
/// 2. `head` が origin に無い（`branches` に、`Branch::origin_name` が `head` と同じ名前を返すものが無い）
///    ときは、`HeadNotOnOrigin`。
pub fn authorize_pull_request_creation(
    origin_url: Option<&str>,
    head: &str,
    branches: &[Branch],
) -> Result<(), PullRequestCreationError> {
    if origin_url
        .and_then(crate::domain::git::GitHubRepository::from_origin_url)
        .is_none()
    {
        return Err(PullRequestCreationError::NotGitHubRepository);
    }
    if !branches
        .iter()
        .any(|branch| branch.origin_name() == Some(head))
    {
        return Err(PullRequestCreationError::HeadNotOnOrigin {
            branch: head.into(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "services_test.rs"]
pub(crate) mod services_tests;
