use super::shared_test_helpers::*;
use super::*;

#[test]
fn test_監視失敗_対象の読取を失敗にして購読へ通知する() {
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let state = state_with_subscriptions(subscriptions);
    handle_watch_failure(&state, "watch unavailable".into());
    assert_eq!(
        state.dirty_count().error.unwrap().message,
        "Watcher(\"watch unavailable\")"
    );
    assert!(matches!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Repository(_)
    ));
}

#[test]
fn test_ファイル監視_gitディレクトリの中の変化では変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/.git/worktrees/feature/index")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/src/main.rs")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}

#[test]
fn test_git監視_他のworktreeのindexの変化ではrootの変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_git_events(
        state.as_ref(),
        Path::new("/repo/.git"),
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
}

#[test]
fn test_git監視_linked_worktreeは自分のindexだけを読み直しrefの変化では読み直さない() {
    // Given
    let state = state_at(
        "/repo-worktrees/feature",
        false,
        crate::test_support::state_subscription::test_subscriptions(),
    );
    let git_dir = Path::new("/repo/.git/worktrees/feature");
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/HEAD"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}
