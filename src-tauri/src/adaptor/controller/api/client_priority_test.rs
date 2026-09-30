use super::*;
use crate::adaptor::gateway::failure_records::FailureRecordStore;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingFailureRecords {
    store: Arc<FailureRecordStore>,
    resolved: AtomicUsize,
}

impl crate::domain::failure::FailureRecordRepository for CountingFailureRecords {
    fn record_observed(
        &self,
        key: &FailureKey,
        failure: WorkFailure,
        requires_attention: bool,
    ) -> bool {
        self.store.record_observed(key, failure, requires_attention)
    }

    fn record_resolved(&self, key: &FailureKey) -> bool {
        self.resolved.fetch_add(1, Ordering::SeqCst);
        self.store.record_resolved(key)
    }
}

#[test]
fn test_枠の拒否_次の受理で一度だけ解く() {
    // Given
    let store = Arc::new(FailureRecordStore::default());
    let records = Arc::new(CountingFailureRecords {
        store: store.clone(),
        resolved: AtomicUsize::new(0),
    });
    let reporter = PriorityFailureReporter::new(Some(Arc::new(FailureRecordingUsecase::new(
        records.clone(),
        None,
    ))));
    let rejection = Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::QueueFull,
    };
    // When / Then
    reporter.admitted();
    assert_eq!(records.resolved.load(Ordering::SeqCst), 0);
    reporter.rejected("/method", &rejection);
    assert!(store.records("daemon")[0].record.active);
    reporter.admitted();
    assert_eq!(records.resolved.load(Ordering::SeqCst), 1);
    reporter.admitted();
    assert_eq!(records.resolved.load(Ordering::SeqCst), 1);
    assert!(!store.records("daemon")[0].record.active);
}

struct PausingFailureRecords {
    store: Arc<FailureRecordStore>,
    resolved: AtomicUsize,
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
        self.resolved.fetch_add(1, Ordering::SeqCst);
        self.store.record_resolved(key)
    }
}

#[test]
fn test_枠の拒否_記録後の並行した受理で一度だけ解く() {
    // Given
    let store = Arc::new(FailureRecordStore::default());
    let (observed, was_observed) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let records = Arc::new(PausingFailureRecords {
        store: store.clone(),
        resolved: AtomicUsize::new(0),
        observed,
        resume: std::sync::Mutex::new(resumed),
    });
    let reporter = Arc::new(PriorityFailureReporter::new(Some(Arc::new(
        FailureRecordingUsecase::new(records.clone(), None),
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
    assert_eq!(records.resolved.load(Ordering::SeqCst), 1);
    reporter.admitted();
    assert_eq!(records.resolved.load(Ordering::SeqCst), 1);
}
