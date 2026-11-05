use super::*;
use crate::domain::failure::TechnicalFailureNature;

#[test]
fn test_実行木読取保持_未使用のfoldを期限で破棄する() {
    // Given
    let records =
        crate::domain::workflow::services::test_helpers::isolated_session_timestamp_facts();
    let (fold, folded) = TreeFold::from_records(&records[0].meta.tree_id, &records)
        .unwrap()
        .unwrap();
    let mut tree = HeldTree {
        fold: Some(HeldFold {
            fold,
            read_through: records.last().unwrap().seq,
            terminal: false,
            view: folded_execution(folded),
            used_at: std::time::Instant::now(),
        }),
        ..Default::default()
    };
    // When / Then
    tree.expire(FOLD_IDLE_LIMIT);
    assert!(tree.fold.is_some());
    tree.fold.as_mut().unwrap().used_at -= std::time::Duration::from_secs(601);
    tree.expire(FOLD_IDLE_LIMIT);
    assert!(tree.fold.is_none());
}

#[test]
fn test_workspace_query_error_corruptをcorrupt_stored_stateへ写像する() {
    // Given
    let correlation_id = "workspace-corrupt-correlation";

    // When
    let error = execution_read_error(crate::domain::local_event::LocalEventQueryError::Corrupt {
        correlation_id: correlation_id.to_string(),
    });

    // Then
    assert_eq!(
        error,
        WorkflowError::CorruptStoredState(format!(
            "store corrupt (correlation_id={correlation_id})"
        ))
    );
}

#[test]
fn test_workspace_query_結果不明と期限切れの分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::LocalEventQueryError;
    use connectrpc::ErrorCode;
    // Given
    for (error, expected) in [
        (
            LocalEventQueryError::CanonicalWriterRequired,
            ErrorCode::Internal,
        ),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
    ] {
        // When / Then
        assert_eq!(execution_read_error(error).connect_code(), expected);
    }
}

#[test]
fn test_実行木読取失敗_書込権限不足とstorageへのアクセス要求を区別する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
    // Given
    let failure = SafeOperationFailure::new(
        SessionOperationFailureKind::StorageUnavailable,
        TechnicalFailureNature::Other,
        "access required",
        "id",
    );
    // When
    let access = execution_read_error(LocalEventQueryError::StorageAccessRequired {
        failure: failure.clone(),
    });
    let writer = execution_read_error(LocalEventQueryError::CanonicalWriterRequired);
    // Then
    assert!(matches!(access, WorkflowError::Store(_)));
    assert_eq!(
        access,
        WorkflowError::Store(LocalEventQueryError::StorageAccessRequired { failure }.into())
    );
    assert!(matches!(writer, WorkflowError::External(_)));
    assert_eq!(writer.connect_code(), connectrpc::ErrorCode::Internal);
}

#[test]
fn test_store問い合わせエラー_停止の分類を保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = execution_read_error(
            crate::domain::local_event::LocalEventQueryError::Technical(stopped.into()),
        );
        // Then
        assert_eq!(
            error.connect_code(),
            crate::domain::failure::TechnicalFailure::from(stopped).connect_code()
        );
        assert!(matches!(error, WorkflowError::Technical(value) if value == stopped.into()));
    }
}
