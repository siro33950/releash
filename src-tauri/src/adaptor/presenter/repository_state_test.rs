use super::*;

#[test]
fn test_スキャン完了通知_購読を無効化する() {
    // Given
    let publisher = crate::adaptor::presenter::state_subscription::test_output();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let notifier = ClientRepositoryStateNotifier::new(publisher);

    // When
    notifier.snapshot_changed(SnapshotNotification {
        worktree_paths: vec!["/repo".into()],
    });

    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Repository(vec!["/repo".into()])
    );
}
