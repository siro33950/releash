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

#[test]
fn test_失敗記録のキー_操作と対象を保持する() {
    // Given
    let operation = "workflow_start";
    let target = "tree";
    // When
    let key = FailureKey::new(operation, target);
    // Then
    assert_eq!(key.operation, operation);
    assert_eq!(key.target, target);
}

#[test]
fn test_作業の失敗_技術的失敗の性質と文面を保持する() {
    // Given
    let failure = TechnicalFailure {
        nature: TechnicalFailureNature::TimedOut,
        message: "timed out".into(),
    };
    // When
    let work = WorkFailure::from(failure);
    // Then
    assert_eq!(
        work.kind,
        Failure::Technical(TechnicalFailureNature::TimedOut)
    );
    assert_eq!(work.message, "timed out");
    assert_eq!(work.to_string(), "timed out");
}
