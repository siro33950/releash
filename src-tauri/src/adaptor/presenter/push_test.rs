use super::*;

#[test]
fn test_push変換_各通知と再同期のメッセージを維持する() {
    let sink = PushSink::new();
    let mut receiver = sink.subscribe();
    for message in [
        PushMessage::FileChange {
            watcher_id: 7,
            path: "/repo/file".into(),
            kind: "change".into(),
        },
        PushMessage::GitStatusChanged {
            repo_path: "/repo".into(),
        },
        PushMessage::ReviewCommentsChanged("/repo".into()),
    ] {
        sink.publish(message);
    }

    let events: Vec<_> = (0..3)
        .map(|_| {
            wire::Push::decode(receiver.try_recv().unwrap().as_ref())
                .unwrap()
                .event
                .unwrap()
        })
        .collect();
    assert_eq!(
        events,
        vec![
            wire::push::Event::FileChange(wire::FileChangeEvent {
                watcher_id: Some(7),
                path: Some("/repo/file".into()),
                kind: Some("change".into()),
            }),
            wire::push::Event::GitStatusChanged(wire::GitStatusChangedEvent {
                repo_path: Some("/repo".into()),
            }),
            wire::push::Event::ReviewCommentsChanged(wire::ResultString {
                value: Some("/repo".into()),
            }),
        ]
    );
    assert_eq!(
        wire::Push::decode(resync().as_ref()).unwrap().event,
        Some(wire::push::Event::Resync(wire::Unit {}))
    );
}
