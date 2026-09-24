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
    let repository =
        LocalProviderLifecycleEventRepository::new(source.clone(), "installation".into());
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
    for (error, expected) in cases {
        let source = Arc::new(FailingRepository {
            error,
            failures: usize::MAX,
            calls: AtomicUsize::new(0),
        });
        let repository =
            LocalProviderLifecycleEventRepository::new(source.clone(), "installation".into());
        // When
        let error = repository
            .resolve_bounded(&CommitIdentity::parse("resolution").unwrap())
            .await
            .unwrap_err();
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
async fn test_確定照会_混雑の上限に達しても元の分類を保持する() {
    // Given
    let source = Arc::new(FailingRepository {
        error: LocalEventQueryError::QueryBusy,
        failures: usize::MAX,
        calls: AtomicUsize::new(0),
    });
    let repository =
        LocalProviderLifecycleEventRepository::new(source.clone(), "installation".into());
    // When
    let error = repository
        .resolve_bounded(&CommitIdentity::parse("resolution").unwrap())
        .await
        .unwrap_err();
    // Then
    assert_eq!(error.failure_kind(), FailureKind::Temporary);
    assert_eq!(source.calls.load(Ordering::SeqCst), 4);
}

#[derive(Default)]
struct ConflictingAppendRepository {
    commits: AtomicUsize,
    reads: AtomicUsize,
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
        _: LocalAtomicBatch,
    ) -> Result<CommitBatchResult, CommitBatchError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        Err(CommitBatchError::StreamHeadConflict {
            current: crate::domain::local_event::StreamVersion::zero(),
        })
    }
    async fn resolve_commit(
        &self,
        _: CommitIdentity,
    ) -> Result<CommitResolution, LocalEventQueryError> {
        panic!("a rejected commit has no unknown outcome")
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
async fn test_lifecycle追記_保存競合は同じ位置で再試行せず上位へ返す() {
    // Given
    let source = Arc::new(ConflictingAppendRepository::default());
    let repository =
        LocalProviderLifecycleEventRepository::new(source.clone(), "installation".into());
    let event = ScopedProviderLifecycleEvent::new(
        ProviderLifecycleScope::new("session").unwrap(),
        crate::domain::provider_lifecycle::ProviderLifecycleEvent::stop_observed("binding")
            .unwrap(),
    );
    // When
    let error = repository.append(vec![event]).await.unwrap_err();
    // Then
    assert_eq!(error.failure_kind(), FailureKind::RestartRequired);
    assert_eq!(source.commits.load(Ordering::SeqCst), 1);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
}
