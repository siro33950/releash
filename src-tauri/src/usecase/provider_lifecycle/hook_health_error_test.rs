use super::*;

#[test]
fn test_hook_healthの競合をその場で再試行可能な失敗に変えない() {
    // Given / When
    let error = map_error(ProviderHookHealthRepositoryError::Conflict);
    // Then
    assert_eq!(error, ProviderHookHealthUsecaseError::Conflict);
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
        ProviderHookHealthRepositoryError::Store(
            (crate::domain::local_event::CommitBatchError::TreeHeadConflict).into(),
        ),
        ProviderHookHealthRepositoryError::StorageUnavailable,
        ProviderHookHealthRepositoryError::Store(
            (crate::domain::local_event::CommitBatchError::QueueBusy).into(),
        ),
        ProviderHookHealthRepositoryError::Store(
            (crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "failure".into(),
            })
            .into(),
        ),
        ProviderHookHealthRepositoryError::Store(
            (crate::domain::local_event::CommitBatchError::PayloadConflict).into(),
        ),
        ProviderHookHealthRepositoryError::Store(
            (crate::domain::local_event::CommitBatchError::Corrupt {
                correlation_id: "id".into(),
            })
            .into(),
        ),
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
            let attempts = if matches!(
                &failure,
                ProviderHookHealthRepositoryError::StorageUnavailable
                    | ProviderHookHealthRepositoryError::Store(
                        crate::domain::failure::StorageFailure {
                            nature: crate::domain::failure::TechnicalFailureNature::Transient,
                            ..
                        }
                    )
            ) {
                4
            } else {
                1
            };
            let expected = match &failure {
                ProviderHookHealthRepositoryError::Conflict => {
                    ProviderHookHealthUsecaseError::Conflict
                }
                ProviderHookHealthRepositoryError::StorageUnavailable => {
                    ProviderHookHealthUsecaseError::StorageUnavailable
                }
                ProviderHookHealthRepositoryError::Store(failure)
                    if failure.nature
                        == crate::domain::failure::TechnicalFailureNature::Transient =>
                {
                    ProviderHookHealthUsecaseError::StorageUnavailable
                }
                ProviderHookHealthRepositoryError::Store(failure) => {
                    ProviderHookHealthUsecaseError::Store(failure.clone())
                }
                error => panic!("unexpected test input: {error:?}"),
            };
            assert_eq!(result.unwrap_err(), expected);
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
