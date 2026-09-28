use super::*;
use crate::adaptor::presenter::client as wire;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_state::worker::InvalidateReason;
use prost::Message;

#[test]
fn test_スキャン完了通知_gitとbranchとfileのみclientへ送る() {
    // Given
    let sink = Arc::new(PushSink::new());
    let mut receiver = sink.subscribe();
    let notifier = ClientRepositoryStateNotifier::new(
        sink,
        crate::adaptor::presenter::state_subscription::test_output(),
    );

    // When
    notifier.snapshot_changed(SnapshotNotification {
        worktree_paths: vec!["/repo".into()],
        file_watcher_ids: vec![7],
        reason: InvalidateReason::file(Some("/repo/file.txt".into())),
    });

    // Then
    let mut events = Vec::new();
    while let Ok(bytes) = receiver.try_recv() {
        events.push(wire::Push::decode(bytes.as_ref()).unwrap().event.unwrap());
    }
    assert_eq!(
        events,
        vec![
            wire::push::Event::GitStatusChanged(wire::GitStatusChangedEvent {
                repo_path: Some("/repo".into()),
            }),
            wire::push::Event::FileChange(wire::FileChangeEvent {
                watcher_id: Some(7),
                path: Some("/repo/file.txt".into()),
                kind: Some("change".into()),
            }),
        ]
    );
}
