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

#[test]
fn test_pr解決_一覧の状態を反映し保持中の内容を残す() {
    use crate::domain::git_host::{Check, CheckBucket, PrComment, PrCommentAuthor};

    // Given
    let held = PullRequest {
        body: "held body".into(),
        checks: Checks::new(vec![Check {
            name: "test".into(),
            bucket: CheckBucket::Pending,
            started_at: Some(1),
            completed_at: None,
            job: None,
        }]),
        conversation: PrConversation::new(
            vec![PrComment {
                author: PrCommentAuthor {
                    login: "author".into(),
                    is_bot: false,
                },
                written_at: 1,
                body: "comment".into(),
            }],
            vec![],
        ),
        ..pr(2)
    };
    let info = PrInfo {
        number: 2,
        url: "listed url".into(),
        state: PrState::Closed,
        draft: true,
    };
    let status = CachedResult {
        value: Some(PrStatus {
            open_prs: vec![],
            completed_prs: vec![("branch".into(), info.clone())],
        }),
        error: None,
    };

    // When
    let resolved = BranchPullRequest::resolve(
        status,
        "branch",
        CachedResult {
            value: Some(held.clone()),
            error: None,
        },
    );

    // Then
    assert_eq!(
        resolved,
        CachedResult {
            value: Some(BranchPullRequest::Present(PullRequest { info, ..held })),
            error: None,
        }
    );
}

#[test]
fn test_pr再取得判定_一覧のブランチの内容が不足するときだけ必要になる() {
    // Given
    let status = CachedResult {
        value: Some(PrStatus {
            open_prs: vec![("branch".into(), pr(2).info)],
            completed_prs: vec![],
        }),
        error: None,
    };
    // When / Then
    for (list, branch, held, expected) in [
        (status.clone(), "branch", None, true),
        (status.clone(), "branch", Some(pr(1)), true),
        (status.clone(), "branch", Some(pr(2)), false),
        (status, "missing", Some(pr(1)), false),
        (CachedResult::default(), "branch", Some(pr(1)), false),
    ] {
        assert_eq!(
            BranchPullRequest::needs_refresh(
                &list,
                branch,
                &CachedResult {
                    value: held,
                    error: None,
                }
            ),
            expected
        );
    }
}
