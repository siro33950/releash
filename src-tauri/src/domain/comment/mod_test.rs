use super::*;

#[test]
fn test_review停止_技術的な失敗の値とメッセージを保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given
    for nature in [
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "technical failure".into(),
        };
        // When
        let error = ReviewError::Technical(failure.clone());
        // Then
        assert_eq!(error.to_string(), "technical failure");
        assert!(matches!(error, ReviewError::Technical(value) if value == failure));
    }
}
