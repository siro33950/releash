use crate::common::operation_context::OperationStopped;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};

#[test]
fn test_停止理由_技術的な失敗の性質とメッセージを保持する() {
    // Given
    for (stopped, nature) in [
        (OperationStopped::Expired, TechnicalFailureNature::TimedOut),
        (
            OperationStopped::Cancelled,
            TechnicalFailureNature::Cancelled,
        ),
    ] {
        // When
        let failure = TechnicalFailure::from(stopped);

        // Then
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, stopped.to_string());
    }
}

#[tokio::test]
async fn test_taskの失敗_取り消しとpanicの性質を区別する() {
    // Given
    let task = tokio::spawn(std::future::pending::<()>());
    task.abort();
    let cancelled = task.await.unwrap_err();
    let panicked = tokio::spawn(async { panic!("task failed") })
        .await
        .unwrap_err();
    // When / Then
    for (error, nature) in [
        (cancelled, TechnicalFailureNature::Cancelled),
        (panicked, TechnicalFailureNature::Other),
    ] {
        let message = error.to_string();
        let failure = TechnicalFailure::from(error);
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, message);
    }
}
