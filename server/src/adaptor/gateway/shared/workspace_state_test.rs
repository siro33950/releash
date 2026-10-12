use super::*;
use crate::domain::local_event::{
    SafeOperationFailure, SessionOperationFailureKind, StreamVersion,
};

#[test]
fn test_ui状態の失敗変換_storeの利用不能を技術的な失敗として返す() {
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        // Given
        let failure = SafeOperationFailure::new(
            SessionOperationFailureKind::StorageUnavailable,
            nature,
            "store unavailable",
            "correlation",
        );
        let query = LocalEventQueryError::StorageUnavailable {
            failure: failure.clone(),
        };
        let commit = CommitBatchError::StorageUnavailable { failure };
        let expected_query = query.to_string();
        let expected_commit = commit.to_string();
        // When / Then
        for (actual, message) in [
            (WorkspaceStateError::from(query), expected_query),
            (WorkspaceStateError::from(commit), expected_commit),
        ] {
            assert!(
                matches!(actual, WorkspaceStateError::Technical(error) if error == TechnicalFailure { nature, message })
            );
        }
    }
}

#[test]
fn test_ui状態の失敗変換_版競合だけを区別し既存の技術的な失敗を保持する() {
    // Given
    let failure = TechnicalFailure {
        nature: TechnicalFailureNature::TimedOut,
        message: "original timeout".into(),
    };
    // When / Then
    for actual in [
        WorkspaceStateError::from(LocalEventQueryError::Technical(failure.clone())),
        WorkspaceStateError::from(CommitBatchError::Technical(failure.clone())),
    ] {
        assert!(matches!(actual, WorkspaceStateError::Technical(error) if error == failure));
    }
    assert!(matches!(
        WorkspaceStateError::from(CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(1).unwrap()
        }),
        WorkspaceStateError::VersionConflict
    ));
    assert!(
        matches!(WorkspaceStateError::from(CommitBatchError::PayloadConflict), WorkspaceStateError::Technical(error) if error.nature == TechnicalFailureNature::Other)
    );
    assert!(
        matches!(WorkspaceStateError::from(LocalEventQueryError::QueryBusy), WorkspaceStateError::Technical(error) if error.nature == TechnicalFailureNature::Transient)
    );
}

#[test]
fn test_ui状態の失敗変換_ioの種別と不正なデータを技術的な失敗へ写す() {
    // Given
    let io = std::io::Error::new(std::io::ErrorKind::TimedOut, "read timed out");
    let json = serde_json::from_str::<bool>("invalid").unwrap_err();
    // When / Then
    assert!(
        matches!(WorkspaceStateError::from(io), WorkspaceStateError::Technical(error) if error.nature == TechnicalFailureNature::TimedOut && error.message == "read timed out")
    );
    assert!(
        matches!(WorkspaceStateError::from(json), WorkspaceStateError::Technical(error) if error.nature == TechnicalFailureNature::Other)
    );
    assert!(
        matches!(unexpected("Unexpected workspace state event"), WorkspaceStateError::Technical(error) if error.nature == TechnicalFailureNature::Other && error.message == "Unexpected workspace state event")
    );
}
