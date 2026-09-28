use super::*;
use crate::adaptor::presenter::client as wire;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_state::worker::InvalidateReason;
use prost::Message;

#[test]
fn test_スキャン完了通知_fileだけをclientへ送り購読を無効化する() {
    // Given
    let sink = Arc::new(PushSink::new());
    let mut receiver = sink.subscribe();
    let publisher = crate::adaptor::presenter::state_subscription::test_output();
    let mut changes = crate::test_support::state_subscription::changes(&publisher);
    let notifier = ClientRepositoryStateNotifier::new(sink, publisher);

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
        vec![wire::push::Event::FileChange(wire::FileChangeEvent {
            watcher_id: Some(7),
            path: Some("/repo/file.txt".into()),
            kind: Some("change".into()),
        })]
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Repository(vec!["/repo".into()])
    );
}
