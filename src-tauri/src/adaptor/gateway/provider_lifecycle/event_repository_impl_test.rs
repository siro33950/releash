use super::*;
use crate::domain::local_event::{
    DomainEventPage, LocalEventQuery, LocalEventQueryError, LocalEventQueryResult,
    SafeOperationFailure, SessionOperationFailureKind,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct FailingRepository {
    error: LocalEventQueryError,
    failures: usize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for FailingRepository {
    async fn commit_batch(
        &self,
        _: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        panic!("not used")
    }
    async fn resolve_commit(
        &self,
        _: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.failures {
            Err(self.error.clone())
        } else {
            Ok(CommitResolution::NotCommitted)
        }
    }
    async fn load_stream(
        &self,
        _: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        Err(self.error.clone())
    }
    async fn query(
        &self,
        _: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        panic!("not used")
    }
}

#[tokio::test]
async fn test_確定照会_混雑のみを同じ位置で再試行する() {
    // Given
    let source = Arc::new(FailingRepository {
        error: LocalEventQueryError::QueryBusy,
        failures: 2,
        calls: AtomicUsize::new(0),
    });
    let repository = test_repository(source.clone());
    let identity = CommitIdentity::parse("resolution").unwrap();
    // When
    let result = repository.resolve_bounded(&identity).await;
    // Then
    assert_eq!(result, Ok(CommitResolution::NotCommitted));
    assert_eq!(source.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_確定照会_恒久失敗と期限切れと上位再試行は即座に分類を保持して返す() {
    // Given
    let cases = [
        (LocalEventQueryError::DeadlineExceeded, FailureKind::Expired),
        (
            LocalEventQueryError::Corrupt {
                correlation_id: "corrupt".into(),
            },
            FailureKind::Corrupt,
        ),
        (
            LocalEventQueryError::Internal {
                correlation_id: "internal".into(),
            },
            FailureKind::Internal,
        ),
        (
            LocalEventQueryError::InvalidRequest,
            FailureKind::InvalidInput,
        ),
        (
            LocalEventQueryError::ResponseTooLarge,
            FailureKind::Capacity,
        ),
        (
            LocalEventQueryError::IncompatibleStoredEvent {
                correlation_id: "version".into(),
            },
            FailureKind::StateRequired,
        ),
        (
            LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    crate::domain::failure::FailureKind::StateRequired,
                    "repair required",
                    "fixed",
                ),
            },
            FailureKind::StateRequired,
        ),
        (
            LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    crate::domain::failure::FailureKind::RestartRequired,
                    "resolve at caller",
                    "unknown",
                ),
            },
            FailureKind::RestartRequired,
        ),
    ];
    for (index, (error, expected)) in cases.into_iter().enumerate() {
        let source = Arc::new(FailingRepository {
            error,
            failures: usize::MAX,
            calls: AtomicUsize::new(0),
        });
        let repository = test_repository(source.clone());
        // When
        let identity = CommitIdentity::parse(&format!("resolution-stop-{index}")).unwrap();
        let error = repository.resolve_bounded(&identity).await.unwrap_err();
        // Then
        assert_eq!(error.failure_kind(), expected);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let error = repository
            .load_expected_heads(&[StreamId::application()])
            .await
            .unwrap_err();
        assert_eq!(error.failure_kind(), expected);
    }
}

#[tokio::test]
async fn test_確定照会_混雑が4回を超えても収束まで再試行する() {
    // Given
    let source = Arc::new(FailingRepository {
        error: LocalEventQueryError::QueryBusy,
        failures: 5,
        calls: AtomicUsize::new(0),
    });
    let repository = test_repository(source.clone());
    // When
    let result = repository
        .resolve_bounded(&CommitIdentity::parse("resolution").unwrap())
        .await
        .unwrap();
    // Then
    assert_eq!(result, CommitResolution::NotCommitted);
    assert_eq!(source.calls.load(Ordering::SeqCst), 6);
}

#[derive(Default)]
struct ConflictingAppendRepository {
    commits: AtomicUsize,
    reads: AtomicUsize,
    unknown: bool,
    resolutions: AtomicUsize,
    identities: Mutex<Vec<CommitIdentity>>,
}

#[async_trait::async_trait]
impl LocalEventTransactionRepository for ConflictingAppendRepository {
    fn canonical_event_batch_identity_v1(
        &self,
        _: &[UncommittedDomainEvent],
    ) -> Result<Vec<u8>, String> {
        Ok(vec![1])
    }
    async fn commit_batch(
        &self,
        batch: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.identities
            .lock()
            .unwrap()
            .push(batch.commit_id.clone());
        let call = self.commits.fetch_add(1, Ordering::SeqCst);
        if self.unknown && call == 0 {
            return Err(CommitBatchError::OutcomeUnknown {
                identity: batch.commit_id,
            });
        }
        if !self.unknown && call < 5 {
            return Err(CommitBatchError::StreamHeadConflict {
                current: crate::domain::local_event::StreamVersion::zero(),
            });
        }
        Ok(CommitBatchResult::Committed(
            crate::domain::local_event::CommittedBatch {
                commit_id: batch.commit_id,
                sequence_range: None,
                stream_heads: vec![],
                event_count: batch.events.len() as i64,
                mutation_count: 0,
                result_hash: [0; 32],
            },
        ))
    }
    async fn resolve_commit(
        &self,
        _: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        assert!(self.unknown);
        if self.resolutions.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    FailureKind::RestartRequired,
                    "reload",
                    "test-restart",
                ),
            })
        } else {
            Ok(CommitResolution::NotCommitted)
        }
    }
    async fn load_stream(
        &self,
        _: LoadStreamRequest,
    ) -> Result<DomainEventPage, LocalEventQueryError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(DomainEventPage {
            events: Vec::new(),
            head: crate::domain::local_event::StreamVersion::zero(),
            next_after: None,
        })
    }
    async fn query(
        &self,
        _: LocalEventQuery,
    ) -> Result<LocalEventQueryResult, LocalEventQueryError> {
        panic!("not used by append")
    }
}

#[tokio::test]
async fn test_lifecycle追記_保存競合は状態を再読込して上位から再試行する() {
    // Given
    let source = Arc::new(ConflictingAppendRepository::default());
    let repository = test_repository(source.clone());
    let event = ScopedProviderLifecycleEvent::new(
        ProviderLifecycleScope::new("session").unwrap(),
        crate::domain::provider_lifecycle::ProviderLifecycleEvent::stop_observed("binding")
            .unwrap(),
    );
    // When
    repository.append(vec![event]).await.unwrap();
    // Then
    assert_eq!(source.commits.load(Ordering::SeqCst), 6);
    assert_eq!(source.reads.load(Ordering::SeqCst), 6);
    let records = repository.queue.records("*").await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.kind, FailureKind::RestartRequired);
    assert_eq!(records[0].record.count, 5);
}

fn test_repository(
    source: Arc<dyn LocalEventTransactionRepository>,
) -> LocalProviderLifecycleEventRepository {
    let mut repository = LocalProviderLifecycleEventRepository::new(
        crate::usecase::work_queue::shared().clone(),
        source,
        "installation".into(),
    );
    repository.queue = crate::usecase::work_queue::WorkQueueUsecase::new(Arc::new(
        crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
    ));
    repository
}

#[tokio::test]
async fn test_確定照会のrestart_追記段階で確定状態とheadを再読込する() {
    for pending in [false, true] {
        let source = Arc::new(ConflictingAppendRepository {
            unknown: true,
            ..Default::default()
        });
        let repository = test_repository(source.clone());
        let event = ScopedProviderLifecycleEvent::new(
            ProviderLifecycleScope::new("session").unwrap(),
            crate::domain::provider_lifecycle::ProviderLifecycleEvent::stop_observed("binding")
                .unwrap(),
        );
        if pending {
            let prepared = repository
                .prepare_commit(std::slice::from_ref(&event))
                .unwrap();
            let key = Sha256::digest([1]).into();
            repository.restore_pending(key, prepared).unwrap();
            source.commits.store(1, Ordering::SeqCst);
        }
        repository.append(vec![event]).await.unwrap();
        assert_eq!(source.resolutions.load(Ordering::SeqCst), 2);
        assert_eq!(
            source.reads.load(Ordering::SeqCst),
            if pending { 1 } else { 2 }
        );
        let identities = source.identities.lock().unwrap();
        assert!(identities.iter().all(|identity| identity == &identities[0]));
        let records = repository.queue.records("*").await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record.kind, FailureKind::RestartRequired);
        assert_eq!(records[0].record.count, 1);
        assert!(repository.pending.lock().unwrap().is_empty());
    }
}
