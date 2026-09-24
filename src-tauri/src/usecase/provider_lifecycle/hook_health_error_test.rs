use super::*;
use crate::domain::failure::FailureKind;

#[test]
fn test_hook_healthの競合をその場で再試行可能な失敗に変えない() {
    // Given / When
    let error = map_error(ProviderHookHealthRepositoryError::Conflict);
    // Then
    assert_eq!(error.failure_kind(), FailureKind::RestartRequired);
}

#[test]
fn test_失敗分類_usecase_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ProviderHookHealthUsecaseError::InvalidInput,
            F::InvalidInput,
        ),
        (
            ProviderHookHealthUsecaseError::StorageUnavailable,
            F::Temporary,
        ),
        (ProviderHookHealthUsecaseError::Corrupt, F::Corrupt),
        (
            ProviderHookHealthUsecaseError::Store(F::Temporary),
            F::Temporary,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Expired),
            F::Expired,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Missing),
            F::Missing,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Permission),
            F::Permission,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Capacity),
            F::Capacity,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Internal),
            F::Internal,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Corrupt),
            F::Corrupt,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Cancelled),
            F::Cancelled,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::Unknown),
            F::Unknown,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            ProviderHookHealthUsecaseError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_失敗分類_query_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ProviderHookHealthFailureQueryError::Unavailable,
            F::Temporary,
        ),
        (ProviderHookHealthFailureQueryError::Corrupt, F::Corrupt),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

struct FailingHealthRepository {
    failure: ProviderHookHealthRepositoryError,
    loads: std::sync::atomic::AtomicUsize,
    saves: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderHookHealthRepository for FailingHealthRepository {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<
        crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        ProviderHookHealthRepositoryError,
    > {
        self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut health = crate::domain::provider_lifecycle::ProviderHookHealth::new(provider);
        health.observe_launch("existing");
        health.take_uncommitted_events();
        Ok(crate::domain::provider_lifecycle::VersionedProviderHookHealth::restored(health, 1))
    }

    async fn save(
        &self,
        _: crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        _: &str,
    ) -> Result<
        crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        ProviderHookHealthRepositoryError,
    > {
        self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(self.failure.clone())
    }
}

#[tokio::test]
async fn test_hook_health記録_一時的失敗だけ4回再試行し分類を保持する() {
    // Given
    for failure in [
        ProviderHookHealthRepositoryError::Conflict,
        ProviderHookHealthRepositoryError::Store(FailureKind::RestartRequired),
        ProviderHookHealthRepositoryError::StorageUnavailable,
        ProviderHookHealthRepositoryError::Store(FailureKind::Temporary),
        ProviderHookHealthRepositoryError::Store(FailureKind::Expired),
        ProviderHookHealthRepositoryError::Store(FailureKind::StateRequired),
        ProviderHookHealthRepositoryError::Store(FailureKind::Corrupt),
    ] {
        for operation in 0..3 {
            let repository = Arc::new(FailingHealthRepository {
                failure: failure.clone(),
                loads: Default::default(),
                saves: Default::default(),
            });
            let usecase = ProviderHookHealthUsecase::new(repository.clone());
            // When
            let result = match operation {
                0 => {
                    usecase
                        .record_launch_with_warning(
                            ProviderKind::Codex,
                            "next",
                            Some(ProviderLifecycleUnavailableReason::LocalApiUnavailable),
                            "request",
                        )
                        .await
                }
                1 => {
                    usecase
                        .record_unavailable(
                            ProviderKind::Codex,
                            "existing",
                            ProviderLifecycleUnavailableReason::LocalApiUnavailable,
                            "request",
                        )
                        .await
                }
                _ => {
                    usecase
                        .record_session_started(ProviderKind::Codex, "existing", "request")
                        .await
                }
            };
            // Then
            let attempts = if failure.failure_kind() == FailureKind::Temporary {
                4
            } else {
                1
            };
            assert_eq!(result.unwrap_err().failure_kind(), failure.failure_kind());
            assert_eq!(
                repository.saves.load(std::sync::atomic::Ordering::SeqCst),
                attempts
            );
            assert_eq!(
                repository.loads.load(std::sync::atomic::Ordering::SeqCst),
                attempts
            );
        }
    }
}
