use super::*;
use crate::domain::failure::FailureKind;
use crate::usecase::work_queue::{WorkFailure, WorkKey};

#[tokio::test]
async fn test_作業列配線_複数のcompositionで失敗状態を共有しない() {
    // Given
    let first = initialize_background_work_for_acceptance();
    let second = initialize_background_work_for_acceptance();
    let directory = tempfile::tempdir().unwrap();
    let _first_runtime = TerminalSurfaceRuntime::new(first.clone(), directory.path().join("first"));
    let _second_runtime =
        TerminalSurfaceRuntime::new(second.clone(), directory.path().join("second"));

    // When
    first
        .observe(
            &WorkKey::new("terminal_checkpoint", "terminal"),
            &WorkFailure {
                kind: FailureKind::StateRequired,
                message: "repair required".into(),
            },
        )
        .await;

    // Then
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(first.records("terminal").await[0].requires_attention);
    assert!(second.records("terminal").await.is_empty());
}
