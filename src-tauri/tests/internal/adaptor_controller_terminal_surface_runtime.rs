use releash_lib::test_support::integration::platform::initialize_background_work_for_acceptance;
use releash_lib::test_support::integration::platform::BusinessFailure;
use releash_lib::test_support::integration::platform::Failure;
use releash_lib::test_support::integration::platform::FailureKey;
use releash_lib::test_support::integration::platform::TerminalSurfaceRuntime;
use releash_lib::test_support::integration::platform::WorkFailure;
use std::sync::Arc;

#[tokio::test]
pub async fn test_背景処理の配線_複数のcompositionで失敗状態を共有しない() {
    // Given
    let first = initialize_background_work_for_acceptance();
    let second = initialize_background_work_for_acceptance();
    let directory = tempfile::tempdir().unwrap();
    let _first_runtime = TerminalSurfaceRuntime::new(first.clone(), directory.path().join("first"));
    let _second_runtime =
        TerminalSurfaceRuntime::new(second.clone(), directory.path().join("second"));

    // When
    first.retrying.failures.observed(
        &FailureKey::new("terminal_checkpoint", "terminal"),
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "repair required".into(),
        },
    );

    // Then
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.failures.records("terminal")[0].requires_attention);
    assert!(second.failures.records("terminal").is_empty());
}
