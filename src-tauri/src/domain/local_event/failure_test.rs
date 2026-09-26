use super::*;
use crate::domain::failure::TechnicalFailureNature as N;

#[test]
fn test_失敗分類_生成元の性質を保存し判断を含めず表示する() {
    // Given
    for kind in [
        SessionOperationFailureKind::StorageUnavailable,
        SessionOperationFailureKind::PersistFailure,
    ] {
        for nature in [N::Transient, N::TimedOut, N::Cancelled, N::Other] {
            // When
            let failure = SafeOperationFailure::new(kind, nature, "reason", "id");
            // Then
            assert_eq!(failure.nature, nature);
            assert_eq!(failure.kind, kind);
            assert_eq!(failure.label.value(), "reason");
            assert_eq!(failure.correlation_id, "id");
            assert_eq!(
                failure.to_string(),
                format!("{kind:?} (nature={nature:?}, correlation_id=id): reason")
            );
        }
    }
}
