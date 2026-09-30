use super::*;
use crate::domain::failure::FailureRecordRepository;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{BusinessFailure, Failure};

fn key(operation: &str, target: &str) -> FailureKey {
    FailureKey::new(operation, target)
}

fn failure(kind: Failure, message: &str) -> WorkFailure {
    WorkFailure {
        kind,
        message: message.into(),
    }
}

#[test]
fn test_失敗記録_repositoryは渡された要対応値を保存する() {
    // Given
    let store = FailureRecordStore::default();
    let key = key("workflow_start", "tree");
    // When
    store.record_observed(
        &key,
        failure(Failure::Business(BusinessFailure::Other), "repair"),
        false,
    );
    // Then
    assert!(!store.records("tree")[0].requires_attention);
    assert!(store.attention_messages("tree").is_empty());
}

#[test]
fn test_失敗記録_文面によらず集約し同じ鍵の古い分類を無効にする() {
    // Given
    let store = FailureRecordStore::default();
    store.observe_at(
        &key("save", "a"),
        failure(
            Failure::Technical(TechnicalFailureNature::Transient),
            "first",
        ),
        1,
    );
    store.observe_at(
        &key("save", "b"),
        failure(
            Failure::Technical(TechnicalFailureNature::Transient),
            "other",
        ),
        2,
    );
    // When
    store.observe_at(
        &key("save", "a"),
        failure(
            Failure::Technical(TechnicalFailureNature::Transient),
            "changed",
        ),
        3,
    );
    store.observe_at(
        &key("save", "a"),
        failure(Failure::Technical(TechnicalFailureNature::Other), "corrupt"),
        4,
    );
    // Then
    let current = store.records("a");
    assert_eq!(current.len(), 2);
    assert_eq!(current[0].record.count, 2);
    assert_eq!(current[0].record.first_observed_ms, 1);
    assert_eq!(current[0].record.last_observed_ms, 3);
    assert_eq!(current[0].record.message, "changed");
    assert!(!current[0].record.active);
    assert_eq!(
        current[1].record.kind,
        Failure::Technical(TechnicalFailureNature::Other)
    );
    assert!(current[1].record.active);
    assert_eq!(store.records("*").len(), 3);
}

#[test]
fn test_失敗記録_解消と再観測でも履歴を保ち最新分類だけを有効にする() {
    let store = FailureRecordStore::default();
    let node = key("workflow_start", "node");
    store.observe_at(
        &node,
        failure(Failure::Business(BusinessFailure::Other), "repair"),
        1,
    );
    assert!(store.resolve(&node));
    assert!(!store.records("node")[0].record.active);
    assert!(!store.resolve(&node));
    store.observe_at(
        &node,
        failure(Failure::Business(BusinessFailure::Other), "again"),
        2,
    );
    assert_eq!(store.records("node")[0].record.count, 2);
    assert!(store.records("node")[0].record.active);
    store.observe_at(
        &node,
        failure(
            Failure::Technical(TechnicalFailureNature::Transient),
            "busy",
        ),
        3,
    );
    let records = store.records("node");
    assert!(!records[0].record.active);
    assert!(records[1].record.active);
}

#[test]
fn test_失敗記録_六種類を別記録として保持し同じ種類の再観測をまとめる() {
    // Given
    let store = FailureRecordStore::default();
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
        store.observe_at(&key("save", "target"), failure(kind, "first"), 1);
        store.observe_at(&key("save", "target"), failure(kind, "latest"), 2);
    }
    // Then
    let records = store.records("target");
    assert_eq!(records.len(), 6);
    for (record, kind) in records.iter().zip(kinds) {
        assert_eq!(record.record.kind, kind);
        assert_eq!(record.record.count, 2);
        assert_eq!(record.record.message, "latest");
    }
    assert_eq!(
        records.iter().filter(|record| record.record.active).count(),
        1
    );
}

#[test]
fn test_要対応の変化_同じ理由の再観測では変わらず理由や分類が変わると変わる() {
    let store = FailureRecordStore::default();
    let tree = key("workflow_recovery", "tree");
    assert!(store.observe(
        &tree,
        failure(Failure::Business(BusinessFailure::Other), "repair")
    ));
    assert!(!store.observe(
        &tree,
        failure(Failure::Business(BusinessFailure::Other), "repair")
    ));
    assert!(store.observe(
        &tree,
        failure(Failure::Business(BusinessFailure::Other), "new reason")
    ));
    assert!(store.observe(
        &tree,
        failure(
            Failure::Technical(TechnicalFailureNature::Cancelled),
            "cancel"
        )
    ));
    assert!(!store.observe(
        &tree,
        failure(
            Failure::Technical(TechnicalFailureNature::Transient),
            "busy"
        )
    ));
    assert!(!store.resolve(&tree));
    store.observe(
        &tree,
        failure(Failure::Technical(TechnicalFailureNature::Other), "broken"),
    );
    assert!(store.resolve(&tree));
}

#[test]
fn test_要対応の理由_有効で要対応の記録の文面だけを返す() {
    let store = FailureRecordStore::default();
    let target = key("provider_session_title", "target");
    store.observe_at(
        &target,
        failure(Failure::Business(BusinessFailure::Other), "old"),
        1,
    );
    store.observe_at(
        &target,
        failure(Failure::Technical(TechnicalFailureNature::Other), "current"),
        2,
    );
    store.observe_at(
        &key("other", "elsewhere"),
        failure(
            Failure::Technical(TechnicalFailureNature::Other),
            "elsewhere",
        ),
        3,
    );
    assert_eq!(store.attention_messages("target"), ["current"]);
    store.resolve(&target);
    assert!(store.attention_messages("target").is_empty());
}
