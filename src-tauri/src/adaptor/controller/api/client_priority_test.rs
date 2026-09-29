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

struct PausingFailureRecords {
    store: Arc<FailureRecordStore>,
    observed: std::sync::mpsc::Sender<()>,
    resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl crate::domain::failure::FailureRecordRepository for PausingFailureRecords {
    fn record_observed(
        &self,
        key: &FailureKey,
        failure: WorkFailure,
        requires_attention: bool,
    ) -> bool {
        let changed = self.store.record_observed(key, failure, requires_attention);
        self.observed.send(()).unwrap();
        self.resume.lock().unwrap().recv().unwrap();
        changed
    }

    fn record_resolved(&self, key: &FailureKey) -> bool {
        self.store.record_resolved(key)
    }
}

#[tokio::test]
async fn test_枠の拒否_記録後の並行した受理で一度だけ解く() {
    // Given
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = crate::test_support::state_subscription::changes(&subscriptions);
    let store = Arc::new(FailureRecordStore::default());
    let (observed, was_observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let reporter = Arc::new(PriorityFailureReporter::new(Some(Arc::new(
        FailureRecordingUsecase::new(
            Arc::new(PausingFailureRecords {
                store: store.clone(),
                observed,
                resume: std::sync::Mutex::new(resumed),
            }),
            Some(subscriptions),
        ),
    ))));
    let rejection = Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::QueueFull,
    };
    let rejecting = {
        let reporter = reporter.clone();
        std::thread::spawn(move || reporter.rejected("/method", &rejection))
    };
    was_observed.recv().unwrap();
    assert!(store.records("daemon")[0].record.active);

    // When
    let admitting = (0..2)
        .map(|_| {
            let reporter = reporter.clone();
            std::thread::spawn(move || reporter.admitted())
        })
        .collect::<Vec<_>>();
    resume.send(()).unwrap();
    rejecting.join().unwrap();
    for admitted in admitting {
        admitted.join().unwrap();
    }

    // Then
    assert!(!store.records("daemon")[0].record.active);
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("daemon".into())
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("daemon".into())
    );
    reporter.admitted();
    assert!(changes.try_recv().is_err());
}
