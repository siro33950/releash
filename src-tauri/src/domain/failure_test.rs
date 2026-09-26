use super::*;

#[test]
fn test_技術的失敗_性質とメッセージを保持する() {
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "failure".into(),
        };
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.to_string(), "failure");
    }
}
