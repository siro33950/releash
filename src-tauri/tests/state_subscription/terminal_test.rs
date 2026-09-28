use releash_lib::terminal_subscription_acceptance::TerminalSubscriptionHarness;
use releash_lib::terminal_surface::{
    initialize_background_work_for_acceptance, TerminalProcessLaunchV1, TerminalSurfaceOwnerV1,
    TerminalSurfaceStreamItemV1,
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_terminal購読の実配線でsnapshotと出力を届ける() {
    // Given
    let data = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let path = cwd.path().to_string_lossy().into_owned();
    let owner = TerminalSurfaceOwnerV1::Workspace {
        workspace_path: path.clone(),
    };
    let runtime = TerminalSubscriptionHarness::new(
        initialize_background_work_for_acceptance(),
        data.path().to_path_buf(),
    );
    runtime
        .get_or_spawn_with_process(
            24,
            80,
            Some(path),
            owner.clone(),
            None,
            TerminalProcessLaunchV1 {
                executable: "/bin/cat".into(),
                arguments: vec![],
                environment: Default::default(),
            },
        )
        .unwrap();

    // When
    let mut subscription = runtime
        .subscribe("integration-input".into(), owner.clone())
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .unwrap()
        .unwrap();
    runtime
        .write(owner.clone(), "integration-output\r")
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(TerminalSurfaceStreamItemV1::Output { data, .. }) =
                subscription.next().await
            {
                if data.contains("integration-output") {
                    break data;
                }
            }
        }
    })
    .await
    .unwrap();

    // Then
    assert!(matches!(
        first,
        TerminalSurfaceStreamItemV1::Snapshot { .. }
    ));
    assert!(output.contains("integration-output"));
    runtime.kill(owner).unwrap();
}
