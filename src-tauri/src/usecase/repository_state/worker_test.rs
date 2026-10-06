pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn reasons_merge_keeps_shutdown() {
        let mut reason = InvalidateReason::change();
        reason.merge(InvalidateReason::shutdown());
        reason.merge(InvalidateReason::change());

        assert!(reason.shutdown);
    }

    #[test]
    fn test_走査のきっかけ_まとめると変わった範囲を合わせる() {
        // Given
        let mut reason = InvalidateReason::files();

        // When
        reason.merge(InvalidateReason::refs());

        // Then
        assert_eq!(reason, InvalidateReason::change());
    }
}
