use super::*;
use crate::domain::failure::{BusinessFailure, TechnicalFailureNature};

#[test]
fn test_失敗記録_文面によらず集約し最古の観測を追い出す() {
    // Given
    let mut records = FailureRecords::new(2);
    records.observe(
        "save",
        "a",
        Failure::Technical(TechnicalFailureNature::Transient),
        "first".into(),
        1,
    );
    records.observe(
        "save",
        "b",
        Failure::Technical(TechnicalFailureNature::Transient),
        "other".into(),
        2,
    );
    // When
    records.observe(
        "save",
        "a",
        Failure::Technical(TechnicalFailureNature::Transient),
        "changed".into(),
        3,
    );
    records.observe(
        "save",
        "a",
        Failure::Technical(TechnicalFailureNature::Other),
        "corrupt".into(),
        4,
    );
    // Then
    let current: Vec<_> = records.records().collect();
    assert_eq!(current.len(), 2);
    assert_eq!(current[0].target, "a");
    assert_eq!(current[0].count, 2);
    assert_eq!(current[0].first_observed_ms, 1);
    assert_eq!(current[0].last_observed_ms, 3);
    assert_eq!(current[0].message, "changed");
    assert_eq!(
        current[1].kind,
        Failure::Technical(TechnicalFailureNature::Other)
    );
    assert_eq!(FailureRecords::new(2).records().count(), 0);
}

#[test]
fn test_失敗記録_解消と再観測でも履歴を保ち最新分類だけを有効にする() {
    let mut records = FailureRecords::new(2);
    records.observe(
        "workflow_start",
        "node",
        Failure::Business(BusinessFailure::Other),
        "repair".into(),
        1,
    );
    assert!(records.resolve("workflow_start", "node"));
    assert!(!records.records().next().unwrap().active);
    records.observe(
        "workflow_start",
        "node",
        Failure::Business(BusinessFailure::Other),
        "again".into(),
        2,
    );
    assert_eq!(records.records().next().unwrap().count, 2);
    assert!(records.records().next().unwrap().active);
    records.observe(
        "workflow_start",
        "node",
        Failure::Technical(TechnicalFailureNature::Transient),
        "busy".into(),
        3,
    );
    assert!(!records.records().next().unwrap().active);
    assert!(records.records().last().unwrap().active);
}

#[test]
fn test_失敗記録_六種類を別記録として保持し同じ種類の再観測をまとめる() {
    // Given
    let mut records = FailureRecords::new(6);
    let kinds = [
        Failure::Business(BusinessFailure::VersionConflict),
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::Transient),
        Failure::Technical(TechnicalFailureNature::TimedOut),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::Other),
    ];
    // When
    for kind in kinds {
        records.observe("save", "target", kind, "first".into(), 1);
        records.observe("save", "target", kind, "latest".into(), 2);
    }
    // Then
    assert_eq!(records.records().count(), 6);
    for (record, kind) in records.records().zip(kinds) {
        assert_eq!(record.kind, kind);
        assert_eq!(record.count, 2);
        assert_eq!(record.message, "latest");
    }
    assert_eq!(records.records().filter(|record| record.active).count(), 1);
}
