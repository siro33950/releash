use super::*;

#[test]
fn test_マージ状態_gitの判定を保持しopenなprがない場合だけprの判定を反映する() {
    // Given
    let status = PrStatus {
        open_prs: Vec::from([(
            "open".into(),
            PrInfo {
                number: 1,
                url: "url".into(),
                state: crate::domain::git_host::PrState::Open,
                draft: false,
            },
        )]),
        completed_prs: vec!["merged".into(), "open".into()]
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
    };
    // When / Then
    assert!(status.branch_is_merged("merged", false));
    assert!(!status.branch_is_merged("open", false));
    assert!(status.branch_is_merged("open", true));
    assert!(status.branch_is_merged("unknown", true));
    assert!(!status.branch_is_merged("unknown", false));
}

#[test]
fn test_pr分類_全状態とdraftを区別する() {
    for (state, draft, expected) in [
        (PrState::Open, false, PrClassification::Open),
        (PrState::Open, true, PrClassification::Draft),
        (PrState::Merged, true, PrClassification::Merged),
        (PrState::Closed, true, PrClassification::Closed),
    ] {
        let pr = PrInfo {
            number: 1,
            url: "url".into(),
            state,
            draft,
        };
        assert_eq!(pr.classification(), expected);
    }
}
#[test]
fn test_pr選択_closedはmerge扱いせずopenを優先する() {
    let closed = PrInfo {
        number: 1,
        url: "closed".into(),
        state: PrState::Closed,
        draft: false,
    };
    let mut status = PrStatus {
        completed_prs: Vec::from([("branch".into(), closed.clone())]),
        ..Default::default()
    };
    assert!(!status.branch_is_merged("branch", false));
    assert_eq!(status.for_branch("branch"), Some(&closed));
    assert_eq!(status.open_for_branch("branch"), None);
    let open = PrInfo {
        number: 2,
        url: "open".into(),
        state: PrState::Open,
        draft: false,
    };
    status.open_prs.push(("branch".into(), open.clone()));
    assert_eq!(status.open_for_branch("branch"), Some(&open));
    assert_eq!(status.for_branch("branch"), Some(&open));
    status
        .completed_prs
        .iter_mut()
        .find(|(name, _)| name == "branch")
        .unwrap()
        .1
        .state = PrState::Merged;
    assert_eq!(status.for_branch("branch"), Some(&open));
}

#[test]
fn test_pr選択_同状態の最後を選びopenとmergedとclosedの優先順位を守る() {
    // Given
    let pr = |number, state| PrInfo {
        number,
        state,
        url: "url".into(),
        draft: false,
    };
    let mut status = PrStatus {
        open_prs: vec![
            ("branch".into(), pr(1, PrState::Open)),
            ("branch".into(), pr(2, PrState::Open)),
        ],
        completed_prs: vec![
            ("branch".into(), pr(3, PrState::Merged)),
            ("branch".into(), pr(4, PrState::Merged)),
            ("branch".into(), pr(5, PrState::Closed)),
            ("branch".into(), pr(6, PrState::Closed)),
        ],
    };
    // When / Then
    assert_eq!(status.for_branch("branch").unwrap().number, 2);
    assert!(!status.branch_is_merged("branch", false));
    status.open_prs.clear();
    assert_eq!(status.for_branch("branch").unwrap().number, 4);
    assert!(status.branch_is_merged("branch", false));
    status
        .completed_prs
        .retain(|(_, pr)| pr.state == PrState::Closed);
    assert_eq!(status.for_branch("branch").unwrap().number, 6);
    assert!(!status.branch_is_merged("branch", false));
}
