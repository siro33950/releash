use super::*;
#[test]
fn test_先行数_commit後は一つ増える() {
    // Given
    let tracking = Tracking {
        upstream: "origin/main".into(),
        ahead: 2,
        behind: 1,
    };
    // When / Then
    assert_eq!(tracking.ahead_after_commit(), 3);
}
