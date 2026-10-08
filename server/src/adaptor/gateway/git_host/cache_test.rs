use super::*;
use crate::domain::git_host::{IssueLabel, Milestone, PrAuthor};

fn sample_issue(number: u64) -> IssueInfo {
    IssueInfo {
        number,
        title: format!("Issue {number}"),
        state: "OPEN".to_string(),
        url: format!("https://github.com/owner/repo/issues/{number}"),
        author: PrAuthor {
            login: "author".to_string(),
        },
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
        labels: vec![IssueLabel {
            name: "bug".to_string(),
            color: "d73a4a".to_string(),
        }],
        assignees: vec![PrAuthor {
            login: "assignee".to_string(),
        }],
        body: "body".to_string(),
        milestone: Some(Milestone {
            title: "M1".to_string(),
        }),
    }
}

#[test]
fn test_pr状態の保持_最後に取れた値をrepositoryごとに返す() {
    // Given
    let statuses = LatestPrStatuses::default();
    let status = PrStatus::default();
    // When
    statuses.record("/repo", Ok(status.clone()));
    // Then
    assert_eq!(statuses.result("/repo").value, Some(status));
    assert!(statuses.result("/other").value.is_none());
}

#[test]
fn issue_cache_returns_stored_value_for_same_key() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(30));

    // When
    IssueCache::record(&cache, "/repo", Ok(vec![sample_issue(1)]));
    // Then

    assert_eq!(
        IssueCache::result(&cache, "/repo").value,
        Some(vec![sample_issue(1)])
    );
    assert!(IssueCache::result(&cache, "/other").value.is_none());
}

#[test]
fn issue_cache_returns_none_for_stale_entry() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

    // When
    IssueCache::record(&cache, "/repo", Ok(vec![sample_issue(1)]));
    // Then

    assert!(IssueCache::result(&cache, "/repo").value.is_none());
}

#[test]
fn store_evicts_stale_entries_before_inserting_new_value() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));

    IssueCache::record(&cache, "/old", Ok(Vec::new()));
    // When
    IssueCache::record(&cache, "/new", Ok(Vec::new()));
    // Then

    let map = cache.entries.lock();
    assert!(!map.contains_key("/old"));
    assert!(map.contains_key("/new"));
}

#[test]
fn test_issueの保持_成功の期限後の失敗でも最後の一覧を残す() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));
    cache.record("/repo", Ok(vec![sample_issue(1)]));
    // When
    cache.record("/repo", Err(GitHostError::External("offline".into())));
    let result = cache.result("/repo");
    // Then
    assert_eq!(result.value, Some(vec![sample_issue(1)]));
    assert_eq!(result.error, Some(GitHostError::External("offline".into())));
}

#[test]
fn test_issueの保持_失敗の期限後も最後の一覧と失敗を残す() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));
    cache.record("/repo", Ok(vec![sample_issue(1)]));
    cache.record("/repo", Err(GitHostError::External("offline".into())));
    // When
    let result = cache.result("/repo");
    // Then
    assert_eq!(result.value, Some(vec![sample_issue(1)]));
    assert_eq!(result.error, Some(GitHostError::External("offline".into())));
}

#[test]
fn test_issueの保持_他repositoryの記録でも期限切れの失敗と一覧を残す() {
    // Given
    let cache = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(0));
    cache.record("/repo", Ok(vec![sample_issue(1)]));
    cache.record("/repo", Err(GitHostError::External("offline".into())));
    // When
    cache.record("/other", Ok(vec![sample_issue(2)]));
    let result = cache.result("/repo");
    // Then
    assert_eq!(result.value, Some(vec![sample_issue(1)]));
    assert_eq!(result.error, Some(GitHostError::External("offline".into())));
}

#[test]
fn test_git情報の保持_lock保持中のpanic後も読み書きできる() {
    // Given
    let issues = InMemoryTtlCache::<Vec<IssueInfo>>::new(CacheTtl::from_secs(30));
    let prs = LatestPrStatuses::default();
    // When
    let issue_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = issues.entries.lock();
        panic!("interrupted writer");
    }));
    let pr_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = prs.entries.lock();
        panic!("interrupted writer");
    }));
    issues.record("/repo", Ok(vec![sample_issue(1)]));
    prs.record("/repo", Ok(PrStatus::default()));
    let issue_result = issues.result("/repo");
    let pr_result = prs.result("/repo");
    // Then
    assert!(issue_panic.is_err());
    assert!(pr_panic.is_err());
    assert_eq!(issue_result.value, Some(vec![sample_issue(1)]));
    assert_eq!(pr_result.value, Some(PrStatus::default()));
}
