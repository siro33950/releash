use super::*;

#[test]
fn test_失敗記録_文面によらず集約し最古の観測を追い出す() {
    // Given
    let mut records = FailureRecords::new(2);
    records.observe("save", "a", FailureKind::Temporary, "first".into(), 1);
    records.observe("save", "b", FailureKind::Temporary, "other".into(), 2);
    // When
    records.observe("save", "a", FailureKind::Temporary, "changed".into(), 3);
    records.observe("save", "a", FailureKind::Corrupt, "corrupt".into(), 4);
    // Then
    let current: Vec<_> = records.records().collect();
    assert_eq!(current.len(), 2);
    assert_eq!(current[0].target, "a");
    assert_eq!(current[0].count, 2);
    assert_eq!(current[0].first_observed_ms, 1);
    assert_eq!(current[0].last_observed_ms, 3);
    assert_eq!(current[0].message, "changed");
    assert_eq!(current[1].kind, FailureKind::Corrupt);
    assert_eq!(FailureRecords::new(2).records().count(), 0);
}

#[test]
fn test_失敗記録_解消と再観測でも履歴を保ち最新分類だけを有効にする() {
    let mut records = FailureRecords::new(2);
    records.observe(
        "workflow_start",
        "node",
        FailureKind::StateRequired,
        "repair".into(),
        1,
    );
    assert!(records.resolve("workflow_start", "node"));
    assert!(!records.records().next().unwrap().active);
    records.observe(
        "workflow_start",
        "node",
        FailureKind::StateRequired,
        "again".into(),
        2,
    );
    assert_eq!(records.records().next().unwrap().count, 2);
    assert!(records.records().next().unwrap().active);
    records.observe(
        "workflow_start",
        "node",
        FailureKind::Temporary,
        "busy".into(),
        3,
    );
    assert!(!records.records().next().unwrap().active);
    assert!(records.records().last().unwrap().active);
}
