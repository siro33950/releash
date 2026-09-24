use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind as F};

#[test]
fn test_失敗分類_生成元の分類を保存し再試行表示を導出する() {
    // Given
    for kind in [
        SessionOperationFailureKind::StorageUnavailable,
        SessionOperationFailureKind::PersistFailure,
        SessionOperationFailureKind::OutcomeUnknown,
    ] {
        for classification in [
            F::Temporary,
            F::RestartRequired,
            F::StateRequired,
            F::Expired,
            F::Corrupt,
            F::Internal,
        ] {
            // When
            let failure = SafeOperationFailure::new(kind, classification, "reason", "id");
            // Then
            assert_eq!(failure.failure_kind(), classification);
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.label.value(), "reason");
            assert_eq!(failure.correlation_id, "id");
            assert_eq!(
                failure.to_string(),
                format!(
                    "{kind:?} (retryable={}, correlation_id=id): reason",
                    classification == F::Temporary
                )
            );
        }
    }
}
