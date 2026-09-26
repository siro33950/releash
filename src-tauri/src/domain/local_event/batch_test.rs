#[test]
fn test_storeの版競合_集約の版の競合だけを識別する() {
    use crate::domain::local_event::{CommitBatchError, StreamVersion};
    // Given / When / Then
    for (error, expected) in [
        (CommitBatchError::TreeHeadConflict, true),
        (
            CommitBatchError::StreamHeadConflict {
                current: StreamVersion::new(2).unwrap(),
            },
            true,
        ),
        (CommitBatchError::PayloadConflict, false),
        (CommitBatchError::QueueBusy, false),
        (CommitBatchError::AppendOutcomeUnknown, false),
        (CommitBatchError::CapacityExceeded, false),
    ] {
        assert_eq!(error.is_version_conflict(), expected);
    }
}
