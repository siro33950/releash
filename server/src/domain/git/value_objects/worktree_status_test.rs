use super::*;
use crate::domain::git::{BranchState, FileStatus, Operation};
fn status(state: BranchState, conflict: bool) -> WorktreeStatus {
    WorktreeStatus::new(
        WorktreeBranch {
            name: "main".into(),
            state,
        },
        RepositoryStatusScan {
            dirty_count: 3,
            status: vec![
                FileStatus {
                    path: "z/a.txt".into(),
                    index_status: "modified".into(),
                    worktree_status: "modified".into(),
                    conflict: None,
                },
                FileStatus {
                    path: "new.txt".into(),
                    index_status: "none".into(),
                    worktree_status: "new".into(),
                    conflict: None,
                },
                FileStatus {
                    path: "conflict.txt".into(),
                    index_status: "modified".into(),
                    worktree_status: "modified".into(),
                    conflict: conflict.then_some(ConflictKind::BothModified),
                },
            ],
        },
        LineChanges {
            staged: HashMap::new(),
            unstaged: HashMap::from([(
                "z/a.txt".into(),
                LineCount {
                    additions: 2,
                    deletions: 3,
                },
            )]),
        },
    )
}
#[test]
fn test_変更一覧_分類と並び順と検索と破棄() {
    // Given
    let state = status(BranchState::Unpublished { unpushed: 1 }, true);
    // When / Then
    assert_eq!(
        state
            .staged()
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["z/a.txt"]
    );
    assert_eq!(
        state
            .unstaged()
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["new.txt", "z/a.txt"]
    );
    assert_eq!(state.conflicts()[0].path, "conflict.txt");
    assert_eq!(
        state.discard_summary(),
        DiscardSummary {
            lines: LineCount {
                additions: 2,
                deletions: 3
            },
            trashed_files: 1
        }
    );
    assert_eq!(
        state
            .discard_plan(&["new.txt".into(), "z/a.txt".into()])
            .unwrap(),
        DiscardPlan {
            restore: vec!["z/a.txt".into()],
            trash: vec!["new.txt".into()]
        }
    );
    assert!(state.discard_plan(&[]).is_err());
    assert!(state.discard_plan(&["missing".into()]).is_err());
    let filtered = state.searching(SearchTerm::new("A.TXT"));
    assert_eq!(filtered.changed_file_count(), 3);
    assert_eq!(filtered.conflict_count(), 1);
    assert!(filtered.conflicts().is_empty());
    assert_eq!(filtered.unstaged().len(), 1);
    assert_eq!(filtered.discard_summary().trashed_files, 0);
    assert!(filtered.discard_plan(&["new.txt".into()]).is_ok());
    assert!(filtered.ensure_can_commit().is_ok());
    assert_eq!(filtered.push_plan(true), Ok(Push::Publish));
    assert!(filtered.ensure_can_continue().is_err());
    assert!(filtered.ensure_can_abort().is_err());
    assert_eq!(filtered.searching(SearchTerm::new("z")).staged().len(), 0);
}
#[test]
fn test_操作受理_競合中は続行不可で中止可能() {
    // Given
    let operation = BranchState::Operating(Operation {
        kind: OperationKind::Merge,
        message: Some("merge".into()),
    });
    let conflicted = status(operation.clone(), true);
    // When / Then
    assert!(conflicted.ensure_can_commit().is_err());
    assert!(conflicted.push_plan(false).is_err());
    assert!(conflicted.ensure_can_continue().is_err());
    assert_eq!(conflicted.ensure_can_abort(), Ok(&OperationKind::Merge));
    assert_eq!(
        status(operation, false).ensure_can_continue(),
        Ok(&OperationKind::Merge)
    );
    let tracking = status(
        BranchState::Tracking(crate::domain::git::Tracking {
            upstream: "origin/main".into(),
            ahead: 1,
            behind: 0,
        }),
        false,
    );
    assert_eq!(tracking.push_plan(false), Ok(Push::ToUpstream));
    assert_eq!(tracking.push_plan(true), Ok(Push::ForceWithLease));
    let empty = WorktreeStatus::new(
        tracking.branch().clone(),
        RepositoryStatusScan {
            status: vec![],
            dirty_count: 0,
        },
        LineChanges::default(),
    );
    assert!(empty.ensure_can_commit().is_err());
}

#[test]
fn test_続行メッセージ_commitの有無に応じて入力を検証する() {
    // Given
    for message in [Some("original".into()), None] {
        let state = status(
            BranchState::Operating(Operation {
                kind: OperationKind::Rebase(crate::domain::git::RebaseProgress {
                    onto: "main".into(),
                    step: 1,
                    total: 2,
                }),
                message: message.clone(),
            }),
            false,
        );
        // When / Then
        assert_eq!(
            state
                .continue_message(" edited ")
                .unwrap()
                .map(|m| m.as_str().to_owned()),
            message.as_ref().map(|_| "edited".to_owned())
        );
        assert_eq!(state.continue_message(" ").is_err(), message.is_some());
    }
    assert!(status(BranchState::Unpublished { unpushed: 0 }, false)
        .continue_message("message")
        .is_err());
}
