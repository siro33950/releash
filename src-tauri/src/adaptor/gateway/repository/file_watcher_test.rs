use super::*;

#[tokio::test]
async fn test_木監視_未作成のディレクトリの生成を検知して変化を通知する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("history");
    let (sender, mut changes) = tokio::sync::mpsc::unbounded_channel();
    let gateway = FileWatcherGateway::new(Arc::new(FileWatcherManager::default()));
    // When
    let id = gateway
        .start_tree(
            path.to_str().unwrap(),
            Arc::new(move || {
                let _ = sender.send(());
            }),
        )
        .unwrap();
    // Then
    changes.try_recv().unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("session.json"), "{}").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), changes.recv())
        .await
        .unwrap()
        .unwrap();
    gateway.stop(id).unwrap();
}
