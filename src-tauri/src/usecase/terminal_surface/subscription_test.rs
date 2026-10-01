use super::*;
use crate::usecase::terminal_surface::application::TerminalSurfaceSnapshotDto;

#[test]
fn test_terminal復元点_終了済みの全項目を購読用出力へ写す() {
    // Given
    let owner = crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
        crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
    )
    .unwrap();
    let mut surface = crate::domain::terminal_surface::entities::TerminalSurface::with_checkpoint(
        1,
        owner,
        Some("Shell".into()),
        crate::domain::terminal_surface::TerminalSurfaceCheckpoint {
            replay: "replay".into(),
            sequence: 17,
            cols: 100,
            rows: 30,
        },
    );
    let expected_key = surface.session_key.clone();
    surface.mark_exited(surface.runtime_generation, Some(9));

    // When
    let output = TerminalSurfaceSnapshotDto::from(surface);

    // Then
    assert_eq!(output.session_key, expected_key);
    assert_eq!(output.replay, "replay");
    assert_eq!(output.sequence, 17);
    assert_eq!(output.cols, 100);
    assert_eq!(output.rows, 30);
    assert!(output.is_exited);
    assert_eq!(output.exit_code, Some(9));
    assert_eq!(output.label.as_deref(), Some("Shell"));
}

#[test]
fn test_terminal復元点_新規surfaceの画面の値を写す() {
    // Given
    let owner = crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
        crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
    )
    .unwrap();
    let surface = crate::domain::terminal_surface::entities::TerminalSurface::new(
        1,
        owner,
        Some("Shell".into()),
    );
    let expected_key = surface.session_key.clone();

    // When
    let output = crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Snapshot(
        surface.into(),
    );

    // Then
    assert!(
        matches!(output, crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem::Snapshot(snapshot)
        if snapshot.session_key == expected_key && snapshot.cols == 80
        && snapshot.rows == 24 && snapshot.label.as_deref() == Some("Shell")
        && !snapshot.is_exited)
    );
}

#[tokio::test]
async fn test_terminal入力識別子_上限を受け付け超過を拒否する() {
    // Given
    let presenter =
        crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new();
    let output = Arc::new(
        crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
            &presenter,
        ),
    );
    let usecase = TerminalSubscriptionUsecase::new(output, None);
    let target = SubscriptionTarget::Terminal(
        crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
            crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
        )
        .unwrap(),
    );
    let at_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES);
    let over_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES + 1);
    // When
    let accepted = usecase
        .start_terminal("client", &target, &at_limit, None)
        .await;
    let rejected = usecase
        .start_terminal("client", &target, &over_limit, None)
        .await;
    // Then
    assert!(matches!(
        accepted,
        Err(StateReadError {
            source: StateReadFailure::Technical(_),
            ..
        })
    ));
    assert!(matches!(
        rejected,
        Err(StateReadError {
            source: StateReadFailure::InvalidTerminalInput,
            ..
        })
    ));
}

#[tokio::test]
async fn test_terminal入力識別子_空白とバイト上限を変えず検査する() {
    // Given
    let presenter =
        crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new();
    let output = Arc::new(
        crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
            &presenter,
        ),
    );
    let usecase = TerminalSubscriptionUsecase::new(output, None);
    let target = SubscriptionTarget::RepositoryPaths;
    // When / Then
    for input in [String::new(), " \t\n".into(), "あ".repeat(43)] {
        let error = usecase
            .start_terminal("client", &target, &input, None)
            .await
            .unwrap_err();
        assert!(matches!(
            error.source,
            StateReadFailure::InvalidTerminalInput
        ));
        assert_eq!(error.message, "Invalid terminal input identity");
    }
    let error = usecase
        .start_terminal("client", &target, &"あ".repeat(42), None)
        .await
        .unwrap_err();
    assert_eq!(error.message, "Not a terminal target");
}

#[test]
fn test_terminalのclient管理_二重openを拒否し閉じた購読の処理報告を拒否する() {
    // Given
    let presenter =
        crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new();
    let output = Arc::new(
        crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
            &presenter,
        ),
    );
    let usecase = TerminalSubscriptionUsecase::new(output, None);
    usecase.open_client("client".into()).unwrap();
    // When / Then
    assert_eq!(
        usecase.open_client("client".into()),
        Err(SubscriptionError::AlreadyExists)
    );
    usecase.close_client("client");
    let error = usecase
        .terminal_processed("client", &SubscriptionTarget::RepositoryPaths, 5000)
        .unwrap_err();
    assert!(matches!(
        error.source,
        StateReadFailure::TerminalSubscriptionEnded
    ));
    assert_eq!(error.message, "Terminal subscription ended");
    usecase.open_client("client".into()).unwrap();
    usecase.schedule_terminal_refresh(vec!["client".into()], SubscriptionTarget::RepositoryPaths);
    assert!(usecase.terminal_resets.lock().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
}
