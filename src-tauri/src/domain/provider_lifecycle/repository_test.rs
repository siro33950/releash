use super::*;

#[test]
fn test_失敗分類_provider_lifecycle_repository_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ProviderLifecycleRepositoryError::InvalidInput,
            F::InvalidInput,
        ),
        (
            ProviderLifecycleRepositoryError::StorageUnavailable,
            F::Temporary,
        ),
        (ProviderLifecycleRepositoryError::Corrupt, F::Corrupt),
        (
            ProviderLifecycleRepositoryError::Store(F::Expired),
            F::Expired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_失敗分類_provider_hook_health_repository_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ProviderHookHealthRepositoryError::InvalidInput,
            F::InvalidInput,
        ),
        (
            ProviderHookHealthRepositoryError::Conflict,
            F::RestartRequired,
        ),
        (
            ProviderHookHealthRepositoryError::StorageUnavailable,
            F::Temporary,
        ),
        (ProviderHookHealthRepositoryError::Corrupt, F::Corrupt),
        (
            ProviderHookHealthRepositoryError::Store(F::Expired),
            F::Expired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
