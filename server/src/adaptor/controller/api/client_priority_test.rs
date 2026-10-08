use super::*;
use crate::adaptor::gateway::failure_records::FailureRecordStore;

#[test]
fn test_枠の拒否_読まれない失敗を記録しない() {
    // Given
    let store = Arc::new(FailureRecordStore::default());
    let reporter = PriorityFailureReporter;
    let rejection = Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::QueueFull,
    };
    // When
    reporter.rejected("/method", &rejection);
    reporter.admitted();
    // Then
    assert!(store.records("daemon").is_empty());
}

#[test]
fn test_枠の拒否_記録の持ち主がなくても扱える() {
    // Given
    let reporter = PriorityFailureReporter;
    let rejection = Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::QueueFull,
    };
    // When
    reporter.rejected("/method", &rejection);
    // Then
    reporter.admitted();
}
