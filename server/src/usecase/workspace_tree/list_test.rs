use super::*;
use crate::domain::git_host::{PrInfo, PrStatus};
use std::collections::HashMap;

fn worktree(path: &str, branch: &str, is_merged: bool) -> Worktree {
    Worktree {
        name: branch.to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        is_main: false,
        is_locked: false,
        is_merged,
    }
}

fn values(path: &str, branch: &str, is_merged: bool) -> WorktreeValues {
    WorktreeValues {
        tracking: Fetched::ready(None),
        worktree: worktree(path, branch, is_merged),
        deleting: false,
        dirty_count: Fetched::ready(0),
    }
}

fn failed_tree(message: &str) -> Fetched<WorkspaceTree> {
    Fetched {
        value: None,
        error: Some(failure(message)),
    }
}

fn failure(message: &str) -> crate::domain::failure::WorkFailure {
    crate::domain::failure::WorkFailure {
        kind: crate::domain::failure::Failure::Technical(
            crate::domain::failure::TechnicalFailureNature::Other,
        ),
        message: message.into(),
    }
}

#[test]
fn test_一覧の合成_prの状態でpr情報とmerge済みを決める() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "open", false),
            values("/a/merged", "merged", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: Fetched::ready(PrStatus {
            open_prs: HashMap::from([(
                "open".into(),
                PrInfo {
                    number: 42,
                    url: "https://example.test/pull/42".into(),
                    state: crate::domain::git_host::PrState::Open,
                    draft: false,
                },
            )]),
            completed_prs: vec!["open".into(), "merged".into()]
                .into_iter()
                .map(|name: String| {
                    (
                        name,
                        PrInfo {
                            number: 1,
                            url: "merged-url".into(),
                            state: crate::domain::git_host::PrState::Merged,
                            draft: false,
                        },
                    )
                })
                .collect(),
        }),
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 3]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert_eq!(
        rows[0].open_pull_request,
        Some(PrInfo {
            number: 42,
            url: "https://example.test/pull/42".into(),
            state: crate::domain::git_host::PrState::Open,
            draft: false,
        })
    );
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert_eq!(
        rows[1].state_pull_request.as_ref().unwrap().state,
        crate::domain::git_host::PrState::Merged
    );
    assert!(rows[2].merged);
}

#[test]
fn test_一覧の合成_prが未取得ならworktreeのmerge済みをそのまま使う() {
    // Given
    let repositories = vec![RepositoryValues {
        path: "/a".into(),
        worktrees: Fetched::ready(vec![
            values("/a", "main", false),
            values("/a/git", "git-merged", true),
        ]),
        pull_requests: Fetched::default(),
    }];

    // When
    let list = compose(repositories, vec![Fetched::default(); 2]);

    // Then
    let rows = list.repositories[0].worktrees.value.as_ref().unwrap();
    assert!(!rows[0].merged);
    assert!(rows[1].merged);
    assert!(rows.iter().all(|row| row.open_pull_request.is_none()));
}

#[test]
fn test_一覧の合成_読めているworktreeの並び順に実行木を割り当て失敗と削除中を保つ() {
    // Given
    let mut deleting = values("/a/two", "two", false);
    deleting.deleting = true;
    deleting.dirty_count = Fetched::ready(3);
    let repositories = vec![
        RepositoryValues {
            path: "/a".into(),
            worktrees: Fetched {
                value: Some(vec![values("/a", "one", false), deleting]),
                error: Some(failure("scan failed")),
            },
            pull_requests: Fetched::default(),
        },
        RepositoryValues {
            path: "/b".into(),
            worktrees: Fetched {
                value: None,
                error: Some(failure("not a repository")),
            },
            pull_requests: Fetched::default(),
        },
        RepositoryValues {
            path: "/c".into(),
            worktrees: Fetched::ready(vec![values("/c", "three", false)]),
            pull_requests: Fetched::default(),
        },
    ];

    // When
    let list = compose(
        repositories,
        vec![failed_tree("one"), failed_tree("two"), failed_tree("three")],
    );

    // Then
    let first = &list.repositories[0];
    assert_eq!(
        first
            .worktrees
            .error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("scan failed")
    );
    let rows = first.worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("one"));
    assert_eq!(rows[1].tree, failed_tree("two"));
    assert!(rows[1].deleting);
    assert_eq!(rows[1].dirty_count, Fetched::ready(3));
    assert_eq!(
        list.repositories[1].worktrees,
        Fetched {
            value: None,
            error: Some(failure("not a repository")),
        }
    );
    let rows = list.repositories[2].worktrees.value.as_ref().unwrap();
    assert_eq!(rows[0].tree, failed_tree("three"));
}

#[test]
fn test_一覧の合成_未コミット数とprの読取失敗を未設定と区別する() {
    // Given
    let mut row = values("/repo", "main", false);
    row.dirty_count = Fetched {
        value: None,
        error: Some(failure("scan failed")),
    };
    let repositories = vec![RepositoryValues {
        path: "/repo".into(),
        worktrees: Fetched::ready(vec![row]),
        pull_requests: Fetched {
            value: None,
            error: Some(failure("PR failed")),
        },
    }];
    // When
    let list = compose(repositories, vec![Fetched::default()]);
    // Then
    let row = &list.repositories[0].worktrees.value.as_ref().unwrap()[0];
    assert_eq!(row.dirty_count.value, None);
    assert_eq!(
        row.dirty_count
            .error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("scan failed")
    );
    assert_eq!(
        row.pull_request_error
            .as_ref()
            .map(|failure| failure.message.as_str()),
        Some("PR failed")
    );
}

#[test]
fn test_一覧の合成_完了済みprとopenのprを別々に保持する() {
    // Given
    for state in [
        crate::domain::git_host::PrState::Merged,
        crate::domain::git_host::PrState::Closed,
    ] {
        let completed = PrInfo {
            number: 42,
            url: "https://example.test/42".into(),
            state,
            draft: false,
        };
        let list = compose(
            vec![RepositoryValues {
                path: "/repo".into(),
                worktrees: Fetched::ready(vec![values("/repo", "feature", false)]),
                pull_requests: Fetched::ready(PrStatus {
                    open_prs: Default::default(),
                    completed_prs: [("feature".into(), completed.clone())].into(),
                }),
            }],
            vec![Fetched::default()],
        );
        // Then
        let row = &list.repositories[0].worktrees.value.as_ref().unwrap()[0];
        assert_eq!(row.open_pull_request, None);
        assert_eq!(row.state_pull_request, Some(completed));
    }
}
