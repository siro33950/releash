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

#[test]
fn test_taskの失敗_取り消しとpanicの性質を区別する() {
    // Given
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let (release, wait) = std::sync::mpsc::channel();
    let occupying = crate::common::operation_context::spawn_blocking(move || wait.recv().unwrap());
    let task = crate::common::operation_context::spawn_blocking(|| ());
    task.abort();
    release.send(()).unwrap();
    runtime.block_on(occupying).unwrap();
    let cancelled = runtime.block_on(task).unwrap_err();
    let panicked = runtime
        .block_on(crate::common::operation_context::spawn_blocking(|| {
            panic!("task failed")
        }))
        .unwrap_err();
    // When / Then
    for (error, nature) in [
        (cancelled, TechnicalFailureNature::Cancelled),
        (panicked, TechnicalFailureNature::Other),
    ] {
        let message = error.to_string();
        let failure = crate::domain::git::GitError::Technical(error.into());
        let crate::domain::git::GitError::Technical(failure) = failure else {
            panic!("expected technical failure");
        };
        assert_eq!(failure.nature, nature);
        assert_eq!(failure.message, message);
    }
}
