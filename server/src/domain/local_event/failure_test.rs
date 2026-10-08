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
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn label_truncates_on_char_boundary() {
        let raw = "あ".repeat(100); // 300 bytes
        let text = BoundedNoticeText::label(&raw);
        assert!(text.value().len() <= NOTICE_LABEL_MAX_BYTES);
        assert!(text.value().ends_with('…'));
        assert!(text
            .value()
            .trim_end_matches('…')
            .chars()
            .all(|c| c == 'あ'));
    }

    #[test]
    fn short_text_is_not_truncated() {
        let text = BoundedNoticeText::label("ok");
        assert_eq!(text.value(), "ok");
    }
}
