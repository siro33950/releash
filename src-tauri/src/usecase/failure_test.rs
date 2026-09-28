use super::*;

#[test]
fn test_要対応判定_六種類の失敗の意味だけから決まる() {
    for (kind, expected) in [
        (Failure::Business(BusinessFailure::VersionConflict), false),
        (Failure::Business(BusinessFailure::Other), true),
        (Failure::Technical(TechnicalFailureNature::Transient), false),
        (Failure::Technical(TechnicalFailureNature::TimedOut), true),
        (Failure::Technical(TechnicalFailureNature::Cancelled), false),
        (Failure::Technical(TechnicalFailureNature::Other), true),
    ] {
        assert_eq!(requires_attention(kind), expected);
    }
}

#[test]
fn test_やり直しの判断_版の競合は読み直し一時的な失敗は続行で他はやり直さない() {
    assert_eq!(
        next_attempt(Failure::Business(BusinessFailure::VersionConflict)),
        Some(AttemptProgress::Reload)
    );
    assert_eq!(
        next_attempt(Failure::Technical(TechnicalFailureNature::Transient)),
        Some(AttemptProgress::Continue)
    );
    for kind in [
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::TimedOut),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::Other),
    ] {
        assert_eq!(next_attempt(kind), None);
    }
}

#[test]
fn test_記録する失敗_作業の失敗は文面をそのまま他はdebug表記を使う() {
    let failure = WorkFailure {
        kind: Failure::Business(BusinessFailure::Other),
        message: "plain".into(),
    };
    assert_eq!(failure.work_failure(), failure);
    let error = crate::domain::workflow::WorkflowError::Conflict("stale".into());
    let recorded = error.work_failure();
    assert_eq!(
        recorded.kind,
        Failure::Business(BusinessFailure::VersionConflict)
    );
    assert_eq!(recorded.message, format!("{error:?}"));
}
