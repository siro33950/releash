use super::*;
use crate::adaptor::gateway::failure_records::FailureRecordStore;
use crate::usecase::state_subscription::StateChangeSource;

#[tokio::test]
async fn test_枠の拒否_次の受理で一度だけ解いて購読を通知する() {
    // Given
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&subscriptions);
    let store = Arc::new(FailureRecordStore::default());
    let reporter = PriorityFailureReporter::new(Some(Arc::new(FailureRecordingUsecase::new(
        store.clone(),
        Some(subscriptions),
    ))));
    let rejection = Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::QueueFull,
    };
    // When / Then
    reporter.admitted();
    assert!(changes.try_recv().is_err());
    reporter.rejected("/method", &rejection);
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("daemon".into())
    );
    assert!(store.records("daemon")[0].record.active);
    reporter.admitted();
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("daemon".into())
    );
    reporter.admitted();
    assert!(changes.try_recv().is_err());
    assert!(!store.records("daemon")[0].record.active);
}
