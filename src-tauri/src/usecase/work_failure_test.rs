use super::*;
use crate::domain::failure::{StorageFailure, TechnicalFailure};
use crate::domain::local_event::{CommitBatchError, LocalEventQueryError, StreamVersion};
use crate::domain::workflow::WorkflowError;
use crate::usecase::work_queue::{next_attempt, AttemptProgress};

#[test]
fn test_作業列の失敗_版競合だけが業務の手順を読み直す() {
    for source in [
        CommitBatchError::TreeHeadConflict,
        CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(2).unwrap(),
        },
    ] {
        let error = WorkflowError::from(source);
        assert_eq!(
            Failure::from(&error),
            Failure::Business(BusinessFailure::VersionConflict)
        );
        assert_eq!(
            next_attempt(Failure::from(&error)),
            Some(AttemptProgress::Reload)
        );
    }
    for source in [
        CommitBatchError::PayloadConflict,
        CommitBatchError::CapacityExceeded,
    ] {
        let error = WorkflowError::from(source);
        assert_eq!(
            Failure::from(&error),
            Failure::Technical(TechnicalFailureNature::Other)
        );
        assert_eq!(next_attempt(Failure::from(&error)), None);
    }
    let error = WorkflowError::Store(StorageFailure::from(
        LocalEventQueryError::CanonicalWriterRequired,
    ));
    assert_eq!(next_attempt(Failure::from(&error)), None);
    let stored_conflict = WorkflowError::Store(StorageFailure::from(WorkflowError::Conflict(
        "head advanced".into(),
    )));
    assert_eq!(
        Failure::from(&stored_conflict),
        Failure::Business(BusinessFailure::VersionConflict)
    );
    assert_eq!(
        next_attempt(Failure::from(&stored_conflict)),
        Some(AttemptProgress::Reload)
    );
    let stored_commit_conflict =
        WorkflowError::Store(StorageFailure::from(CommitBatchError::TreeHeadConflict));
    assert_eq!(
        Failure::from(&stored_commit_conflict),
        Failure::Business(BusinessFailure::VersionConflict)
    );
}

#[test]
fn test_作業列の失敗_技術的な性質を保ち一時的な失敗だけ続行する() {
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let error = WorkflowError::Store(
            TechnicalFailure {
                nature,
                message: "operation failed".into(),
            }
            .into(),
        );
        let failure = Failure::from(&error);
        assert_eq!(failure, Failure::Technical(nature));
        assert_eq!(
            next_attempt(failure),
            if nature == TechnicalFailureNature::Transient {
                Some(AttemptProgress::Continue)
            } else {
                None
            }
        );
    }
}

#[test]
fn test_結果不明の追記_技術的な再確認を続け元の失敗を保持する() {
    let error = WorkflowError::from(CommitBatchError::AppendOutcomeUnknown);
    assert_eq!(
        Failure::from(&error),
        Failure::Technical(TechnicalFailureNature::Transient)
    );
    assert_eq!(
        next_attempt(Failure::from(&error)),
        Some(AttemptProgress::Continue)
    );
    assert!(matches!(
        error,
        WorkflowError::Store(crate::domain::failure::StorageFailure {
            source: crate::domain::failure::StorageFailureSource::Commit(
                CommitBatchError::AppendOutcomeUnknown
            ),
            ..
        })
    ));
}

#[test]
fn test_技術的な失敗の包み_全ての性質を同じ値で保持する() {
    use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
    // Given / When / Then
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let technical = TechnicalFailure {
            nature,
            message: "failed".into(),
        };
        let safe = SafeOperationFailure::new(
            SessionOperationFailureKind::PersistFailure,
            nature,
            "failed",
            "id",
        );
        assert_eq!(Failure::from(&technical), Failure::Technical(nature));
        assert_eq!(Failure::from(&safe), Failure::Technical(nature));
    }
}
