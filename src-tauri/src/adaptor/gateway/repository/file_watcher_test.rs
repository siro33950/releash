use crate::adaptor::gateway::repository::file_watcher::file_change_event_from_path;
use std::path::Path;

#[test]
fn test_変更監視_startの返却idが実際の変更通知へ渡る() {
    use super::*;
    use tauri::Manager;
    // Given
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    app.manage(Arc::new(crate::infrastructure::push::PushSink::new()));
    let gateway = FileWatcherGateway::new(
        Arc::new(FileWatcherManager::default()),
        app.handle().clone(),
    );
    let directory = tempfile::tempdir().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    use tauri::Listener;
    app.listen("file-change", move |event| {
        sender.send(event.payload().to_string()).unwrap();
    });
    // When
    let id = gateway.start(directory.path().to_str().unwrap()).unwrap();
    let path = directory.path().join("changed.txt");
    std::fs::write(&path, "changed").unwrap();
    let event: serde_json::Value = serde_json::from_str(
        &receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    )
    .unwrap();
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
