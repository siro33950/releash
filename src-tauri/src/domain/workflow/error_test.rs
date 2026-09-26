use super::*;

#[test]
fn test_storeの版競合_workflowの業務上の競合として保持する() {
    use crate::domain::local_event::{CommitBatchError, StreamVersion};
    // Given
    for source in [
        CommitBatchError::TreeHeadConflict,
        CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(2).unwrap(),
        },
    ] {
        let reason = source.to_string();
        // When
        let error = WorkflowError::from(source);
        // Then
        assert!(matches!(error, WorkflowError::Conflict(message) if message.contains(&reason)));
    }
}

#[test]
fn test_書込失敗_workflow変換後も分類を保持する() {
    use crate::domain::local_event::CommitBatchError;
    // Given
    for error in [
        CommitBatchError::QueueBusy,
        CommitBatchError::CapacityExceeded,
        CommitBatchError::TreeHeadConflict,
        CommitBatchError::AppendOutcomeUnknown,
    ] {
        let expected = error.clone();
        // When / Then
        let actual = WorkflowError::from(error);
        if expected == CommitBatchError::TreeHeadConflict {
            assert!(matches!(actual, WorkflowError::Conflict(_)));
        } else {
            assert!(
                matches!(actual, WorkflowError::Store(failure) if failure.source == crate::domain::failure::StorageFailureSource::Commit(expected))
            );
        }
    }
}

#[test]
fn test_workflow停止_分類とメッセージを保持する() {
    use crate::domain::failure::TechnicalFailure;
    // Given
    for stopped in [
        TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
            message: "Operation deadline exceeded".into(),
        },
        TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
            message: "Operation cancelled".into(),
        },
    ] {
        // When
        let error = WorkflowError::Technical(stopped.clone());
        // Then
        assert_eq!(error, WorkflowError::Technical(stopped.clone()));
        assert_eq!(error.to_string(), stopped.to_string());
    }
}
