use super::*;

#[test]
fn test_失敗分類_commit_batch_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (CommitBatchError::PayloadConflict, F::StateRequired),
        (CommitBatchError::TreeHeadConflict, F::RestartRequired),
        (CommitBatchError::AppendOutcomeUnknown, F::RestartRequired),
        (CommitBatchError::QueueBusy, F::Temporary),
        (
            CommitBatchError::StreamHeadConflict {
                current: StreamVersion::new(1).unwrap(),
            },
            F::RestartRequired,
        ),
        (
            CommitBatchError::OutcomeUnknown {
                identity: CommitIdentity::parse("commit").unwrap(),
            },
            F::RestartRequired,
        ),
        (CommitBatchError::CapacityExceeded, F::Capacity),
        (CommitBatchError::SequenceExhausted, F::Capacity),
        (
            CommitBatchError::Corrupt {
                correlation_id: "id".into(),
            },
            F::Corrupt,
        ),
        (
            CommitBatchError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                    F::Expired,
                    "busy",
                    "id",
                ),
            },
            F::Expired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
