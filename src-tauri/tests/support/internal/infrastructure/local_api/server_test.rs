use std::error::Error;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::*;

#[tokio::test]
pub async fn test_クライアントtoken_masterと分離しdiscoveryへ書き込まない() {
    // Given / When
    let directory = tempfile::tempdir().unwrap();
    let binding =
        crate::infrastructure::local_api::test_binding(directory.path().to_path_buf()).unwrap();
    assert!(!binding.test_discovery_path().exists());
    let master = binding.bearer_token();
    let client = binding.terminal_bearer_token();
    let server = binding
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    server.publish_discovery().unwrap();
    let discovery = std::fs::read_to_string(directory.path().join("local-api.json")).unwrap();
    // Then
    assert_ne!(client, master);
    assert!(discovery.contains(master.as_ref()));
    assert!(!discovery.contains(client.as_ref()));
}

#[tokio::test]
pub async fn test_local_api_server起動_discovery作成失敗を専用errorで返す() {
    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("not-a-directory");
    std::fs::write(&data_path, "occupied").unwrap();

    let server = crate::infrastructure::local_api::test_binding(data_path)
        .unwrap()
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    let error = server.publish_discovery().unwrap_err();

    assert!(matches!(error, LocalApiServerError::Discovery(_)));
    assert!(error.source().is_some());
}

#[tokio::test]
pub async fn test_local_api_server終了_停止を通知して所有discoveryを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let binding =
        crate::infrastructure::local_api::test_binding(directory.path().to_path_buf()).unwrap();
    let discovery_path = binding.test_discovery_path().to_path_buf();
    let server = binding
        .start(Router::new(), &tokio::runtime::Handle::current())
        .inspect(|server| {
            server.publish_discovery().unwrap();
        })
        .unwrap();

    assert!(discovery_path.exists());
    server.shutdown_and_wait().await.unwrap();
    assert!(!discovery_path.exists());
    assert!(!directory.path().join("client-api.json").exists());
}

#[tokio::test]
pub async fn test_local_api_server終了_timeout時にtaskをabortして待機する() {
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task_dropped = Arc::clone(&dropped);
    let task = tokio::spawn(async move {
        let _drop_flag = DropFlag(task_dropped);
        started_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    started_rx.await.unwrap();

    wait_for_server_task(task, Duration::from_millis(1))
        .await
        .unwrap();

    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
pub async fn test_クライアントdiscovery_作成失敗時はmasterの公開を取り消す() {
    let directory = tempfile::tempdir().unwrap();
    // Given
    std::fs::create_dir(directory.path().join("client-api.json")).unwrap();
    // When / Then
    let server = crate::infrastructure::local_api::test_binding(directory.path().to_owned())
        .unwrap()
        .start(Router::new(), &tokio::runtime::Handle::current())
        .unwrap();
    assert!(server.publish_discovery().is_err());
    assert!(!directory.path().join("local-api.json").exists());
}
