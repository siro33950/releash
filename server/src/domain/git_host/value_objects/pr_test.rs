use super::*;

#[test]
fn test_マージ状態_gitの判定を保持しopenなprがない場合だけprの判定を反映する() {
    // Given
    let status = PrStatus {
        open_prs: HashMap::from([(
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
        completed_prs: HashMap::from([("branch".into(), closed.clone())]),
        ..Default::default()
    };
    assert!(!status.branch_is_merged("branch", false));
    assert_eq!(status.for_branch("branch"), Some(&closed));
    let open = PrInfo {
        number: 2,
        url: "open".into(),
        state: PrState::Open,
        draft: false,
    };
    status.open_prs.insert("branch".into(), open.clone());
    assert_eq!(status.for_branch("branch"), Some(&open));
    status.completed_prs.get_mut("branch").unwrap().state = PrState::Merged;
    assert_eq!(status.for_branch("branch"), Some(&open));
}
