use crate::adaptor::gateway::repository::file_watcher::file_change_event_from_path;
use std::path::Path;

#[tokio::test]
async fn test_変更監視_startの返却idが実際の変更通知へ渡る() {
    use super::*;
    // Given
    let sink = Arc::new(crate::infrastructure::push::PushSink::new());
    let mut receiver = sink.subscribe();
    let gateway = FileWatcherGateway::new(Arc::new(FileWatcherManager::default()), sink);
    let directory = tempfile::tempdir().unwrap();
    // When
    let id = gateway.start(directory.path().to_str().unwrap()).unwrap();
    let path = directory.path().join("changed.txt");
    std::fs::write(&path, "changed").unwrap();
    use crate::adaptor::protocol::client as wire;
    use prost::Message;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let push = wire::Push::decode(frame.as_ref()).unwrap();
    let (name, event) = push.into_value().unwrap();
    assert_eq!(name, "file-change");
    gateway.stop(id).unwrap();
    // Then
    assert_eq!(event["watcher_id"], id);
    assert_eq!(
        event["path"],
        path.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(event["kind"], "change");
}

#[test]
fn test_変更通知_既存pathを正規化する() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "content").unwrap();

    let event = file_change_event_from_path(42, &file_path);

    assert_eq!(event.watcher_id, 42);
    assert_eq!(event.kind, "change");
    assert_eq!(
        event.path,
        file_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string()
    );
}

#[test]
fn test_変更通知_削除済みファイル名を保持する() {
    let dir = tempfile::TempDir::new().unwrap();
    let file_path = dir.path().join("deleted.txt");

    let event = file_change_event_from_path(7, &file_path);

    assert_eq!(event.watcher_id, 7);
    assert!(event.path.ends_with("deleted.txt"));
    assert!(!event.path.contains(".."));
}

#[test]
fn test_変更通知_存在しないpathをスラッシュへ統一する() {
    let event = file_change_event_from_path(8, Path::new(r"C:\missing\file.txt"));

    assert_eq!(event.path, "C:/missing/file.txt");
}

#[tokio::test]
async fn test_履歴監視_未作成のディレクトリの生成を検知して購読へ通知する() {
    use super::*;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("history");
    let publisher = crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
    let mut changes = publisher.subscribe_changes();
    let gateway = FileWatcherGateway::new(
        Arc::new(FileWatcherManager::default()),
        Arc::new(crate::infrastructure::push::PushSink::new()),
    )
    .with_state_publisher(publisher);
    let id = gateway.start_tree(path.to_str().unwrap()).unwrap();
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::domain::state_subscription::StateChangeSource::ProviderHistory
    );
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("session.json"), "{}").unwrap();
    let change = tokio::time::timeout(std::time::Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        change,
        crate::domain::state_subscription::StateChangeSource::ProviderHistory
    );
    gateway.stop(id).unwrap();
}
