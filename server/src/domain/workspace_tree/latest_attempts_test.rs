use super::*;

#[test]
fn test_最新試行_実行木と名前と親スコープごとに最大の試行を選ぶ() {
    // Given
    let lane0 = ExecutionParentRef::fanout_child("fanout", Some(0), 0);
    let lane1 = ExecutionParentRef::fanout_child("fanout", Some(1), 0);
    let scopes = [
        AttemptScope {
            execution: "a",
            name: "session",
            parent: Some(&lane0),
        },
        AttemptScope {
            execution: "a",
            name: "session",
            parent: Some(&lane1),
        },
        AttemptScope {
            execution: "b",
            name: "session",
            parent: Some(&lane0),
        },
        AttemptScope {
            execution: "a",
            name: "command",
            parent: Some(&lane0),
        },
    ];
    // When
    let latest = LatestAttempts::new([
        (scopes[0], 2),
        (scopes[0], 1),
        (scopes[1], 1),
        (scopes[2], 1),
        (scopes[3], 1),
    ]);
    // Then
    assert!(!latest.contains(scopes[0], 1));
    assert!(latest.contains(scopes[0], 2));
    for scope in &scopes[1..] {
        assert!(latest.contains(*scope, 1));
    }
}
