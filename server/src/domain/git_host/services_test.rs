pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn issue_branch_name_uses_existing_template() {
        assert_eq!(issue_branch_name(1302), "feat/issues/1302");
    }
}

#[test]
fn test_pr作成許可_githubとoriginブランチを順に確かめる() {
    use super::*;
    // Given
    let branches = vec![Branch::remote("origin/topic")];
    // When / Then
    assert_eq!(
        authorize_pull_request_creation(None, "topic", &branches),
        Err(PullRequestCreationError::NotGitHubRepository)
    );
    assert_eq!(
        authorize_pull_request_creation(Some("https://example.com/a/b"), "topic", &branches),
        Err(PullRequestCreationError::NotGitHubRepository)
    );
    assert_eq!(
        authorize_pull_request_creation(Some("https://github.com/a/b"), "other", &branches),
        Err(PullRequestCreationError::HeadNotOnOrigin {
            branch: "other".into()
        })
    );
    assert_eq!(
        authorize_pull_request_creation(Some("git@github.com:a/b.git"), "topic", &branches),
        Ok(())
    );
}
