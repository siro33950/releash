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
