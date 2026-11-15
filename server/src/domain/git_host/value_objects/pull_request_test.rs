use super::*;
use crate::domain::git_host::{GitHostError, PrState};
fn pr(number: u64) -> PullRequest {
    PullRequest {
        info: PrInfo {
            number,
            url: "url".into(),
            state: PrState::Open,
            draft: false,
        },
        title: "title".into(),
        body: "".into(),
        checks: Checks::default(),
        conversation: PrConversation::default(),
    }
}
#[test]
fn test_pr入力_空タイトルを拒み各入力を保持する() {
    // Given / When / Then
    assert_eq!(
        NewPullRequest::new("".into(), "".into(), "main".into(), false),
        Err(PullRequestCreationError::EmptyTitle)
    );
    let new = NewPullRequest::new(" title ".into(), "".into(), "main".into(), true).unwrap();
    assert_eq!(
        (new.title(), new.body(), new.base(), new.draft()),
        (" title ", "", "main", true)
    );
}
#[test]
fn test_base選択_originのみ名前順で既定を選ぶ() {
    // Given
    let branches = vec![
        Branch::local("local"),
        Branch::remote("upstream/other"),
        Branch::remote("origin/z"),
        Branch::remote("origin/main"),
    ];
    // When / Then
    let bases = PullRequestBases::new(&branches, Some("main".into()));
    assert_eq!(bases.branches, vec!["main", "z"]);
    assert_eq!(bases.default_branch.as_deref(), Some("main"));
    assert_eq!(
        PullRequestBases::new(&branches, Some("local".into())).default_branch,
        None
    );
}
#[test]
fn test_pr解決_一覧と保持した番号を照合し失敗の優先順を守る() {
    // Given
    let failure = GitHostError::External("list".into());
    let held_failure = GitHostError::External("detail".into());
    let status = CachedResult {
        value: Some(PrStatus {
            open_prs: vec![("branch".into(), pr(2).info)],
            completed_prs: vec![],
        }),
        error: None,
    };
    let held = CachedResult {
        value: Some(pr(2)),
        error: Some(held_failure.clone()),
    };
    // When / Then
    assert_eq!(
        BranchPullRequest::resolve(CachedResult::default(), "branch", held.clone()).value,
        None
    );
    assert_eq!(
        BranchPullRequest::resolve(status.clone(), "missing", held.clone()),
        CachedResult {
            value: Some(BranchPullRequest::Absent),
            error: None
        }
    );
    assert_eq!(
        BranchPullRequest::resolve(status.clone(), "branch", held.clone()),
        CachedResult {
            value: Some(BranchPullRequest::Present(pr(2))),
            error: Some(held_failure)
        }
    );
    assert_eq!(
        BranchPullRequest::resolve(
            status.clone(),
            "branch",
            CachedResult {
                value: Some(pr(1)),
                error: None
            }
        )
        .value,
        None
    );
    assert_eq!(
        BranchPullRequest::resolve(
            CachedResult {
                error: Some(failure.clone()),
                ..status
            },
            "branch",
            held
        )
        .error,
        Some(failure)
    );
}
