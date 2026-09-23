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
            },
        )]),
        merged_branches: vec!["merged".into(), "open".into()],
    };
    // When / Then
    assert!(status.branch_is_merged("merged", false));
    assert!(!status.branch_is_merged("open", false));
    assert!(status.branch_is_merged("open", true));
    assert!(status.branch_is_merged("unknown", true));
    assert!(!status.branch_is_merged("unknown", false));
}
