use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Files(Mutex<Vec<String>>);
impl FileWatchGateway for Files {
    fn release(&self, id: u64) {
        self.0.lock().unwrap().push(id.to_string());
    }
    fn start(&self, path: &str) -> Result<u64, String> {
        self.0.lock().unwrap().push(path.into());
        if path == "/missing" {
            Err("missing path".into())
        } else {
            Ok(42)
        }
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.0.lock().unwrap().push(watcher_id.to_string());
        if watcher_id == 42 {
            Ok(())
        } else {
            Err("unknown watcher".into())
        }
    }
}

#[test]
fn test_監視_repository外ではfile_gatewayに引数と結果を委譲する() {
    // Given
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(None, files.clone());
    // When / Then
    assert_eq!(usecase.start("/file").unwrap(), 42);
    usecase.stop(42).unwrap();
    assert!(
        matches!(usecase.start("/missing"), Err(UsecaseError::File(message)) if message == "missing path")
    );
    assert!(
        matches!(usecase.stop(999), Err(UsecaseError::File(message)) if message == "unknown watcher")
    );
    assert_eq!(*files.0.lock().unwrap(), ["/file", "42", "/missing", "999"]);
    assert!(usecase.start_git_dir("/repo").is_err());
}

#[tokio::test]
async fn test_監視_repositoryの購読と解除ではfile_gatewayを呼ばない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let repository = Arc::new(crate::usecase::repository_state::service::tests::watching_service());
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(Some(repository), files.clone());
    // When / Then
    let file = usecase.start(path).unwrap();
    let git = usecase.start_git_dir(path).unwrap();
    usecase.stop(file).unwrap();
    usecase.stop(git).unwrap();
    assert!(files.0.lock().unwrap().is_empty());
    assert!(usecase.start("/missing-watcher-path").is_err());
    assert!(files.0.lock().unwrap().is_empty());
    assert!(
        matches!(usecase.stop(u64::MAX), Err(UsecaseError::File(message)) if message == "unknown watcher")
    );
}

#[derive(Default)]
pub(crate) struct SubscriptionFiles {
    pub(crate) next: std::sync::atomic::AtomicU64,
    pub(crate) active: Mutex<std::collections::HashSet<u64>>,
    pub(crate) fail_stop: std::sync::atomic::AtomicBool,
}
impl FileWatchGateway for SubscriptionFiles {
    fn release(&self, id: u64) {
        self.active.lock().unwrap().remove(&id);
    }
    fn start(&self, path: &str) -> Result<u64, String> {
        if path == "/missing" {
            return Err("missing path".into());
        }
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.active.lock().unwrap().insert(id);
        Ok(id)
    }
    fn stop(&self, id: u64) -> Result<(), String> {
        if self.fail_stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("stop failed".into());
        }
        self.active.lock().unwrap().remove(&id);
        Ok(())
    }
}

#[tokio::test]
async fn test_購読監視_開始失敗と停止失敗を返し成功した停止と購読終了で解放する() {
    // Given
    let files = Arc::new(SubscriptionFiles::default());
    let usecase = Arc::new(WatcherUsecase::new(None, files.clone()));
    let subscription = usecase.subscribe("push".into()).unwrap();
    // When / Then
    assert!(usecase.watch("missing", "/file", false).is_err());
    assert!(usecase.watch("push", "/missing", false).is_err());
    assert!(files.active.lock().unwrap().is_empty());
    let id = usecase.watch("push", "/file", false).unwrap();
    files
        .fail_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        matches!(usecase.stop(id), Err(UsecaseError::File(message)) if message == "stop failed")
    );
    assert!(files.active.lock().unwrap().contains(&id));
    files
        .fail_stop
        .store(false, std::sync::atomic::Ordering::SeqCst);
    usecase.stop(id).unwrap();
    assert!(files.active.lock().unwrap().is_empty());
    usecase.watch("push", "/file", false).unwrap();
    drop(subscription);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !files.active.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(usecase.watch("push", "/file", false).is_err());
}

struct BlockingFiles {
    files: SubscriptionFiles,
    started: tokio::sync::Notify,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    block_stop: bool,
}
impl FileWatchGateway for BlockingFiles {
    fn release(&self, id: u64) {
        self.files.release(id);
    }
    fn start(&self, path: &str) -> Result<u64, String> {
        if path == "/blocked" && !self.block_stop {
            self.started.notify_one();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
        }
        self.files.start(path)
    }
    fn stop(&self, id: u64) -> Result<(), String> {
        if id == 0 && self.block_stop {
            self.started.notify_one();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
        }
        self.files.stop(id)
    }
}

#[tokio::test]
async fn test_監視購読_生成停止がブロックしても別worktreeとpush購読を待たせない() {
    for block_stop in [false, true] {
        // Given
        let (release, receiver) = std::sync::mpsc::channel();
        let files = Arc::new(BlockingFiles {
            files: SubscriptionFiles::default(),
            started: tokio::sync::Notify::new(),
            release: Mutex::new(receiver),
            block_stop,
        });
        let usecase = Arc::new(WatcherUsecase::new(None, files.clone()));
        let subscription = usecase.subscribe("first".into()).unwrap();
        if block_stop {
            usecase.watch("first", "/first", false).unwrap();
        }
        let blocked = {
            let usecase = usecase.clone();
            tokio::task::spawn_blocking(move || {
                if block_stop {
                    usecase.stop(0).map(|_| 0)
                } else {
                    usecase.watch("first", "/blocked", false)
                }
            })
        };
        files.started.notified().await;
        // When / Then
        let another = {
            let usecase = usecase.clone();
            tokio::task::spawn_blocking(move || {
                let subscription = usecase.subscribe("second".into()).unwrap();
                usecase.watch("second", "/second", false).unwrap();
                subscription
            })
        };
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), another)
            .await
            .expect("watch I/O must not hold the shared subscription lock")
            .unwrap();
        release.send(()).unwrap();
        blocked.await.unwrap().unwrap();
        drop(subscription);
        drop(second);
    }
}

#[tokio::test]
async fn test_監視購読_生成中に終了した購読のwatcherは同じidの再購読へ移さず停止する() {
    // Given
    let (release, receiver) = std::sync::mpsc::channel();
    let files = Arc::new(BlockingFiles {
        files: SubscriptionFiles::default(),
        started: tokio::sync::Notify::new(),
        release: Mutex::new(receiver),
        block_stop: false,
    });
    let usecase = Arc::new(WatcherUsecase::new(None, files.clone()));
    let first = usecase.subscribe("same".into()).unwrap();
    let watching = {
        let usecase = usecase.clone();
        tokio::task::spawn_blocking(move || usecase.watch("same", "/blocked", false))
    };
    files.started.notified().await;
    // When
    files
        .files
        .fail_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    drop(first);
    let second = usecase.subscribe("same".into()).unwrap();
    release.send(()).unwrap();
    // Then
    assert!(matches!(
        watching.await.unwrap(),
        Err(UsecaseError::Subscription(WatchSubscriptionError::NotFound))
    ));
    assert!(files.files.active.lock().unwrap().is_empty());
    let id = usecase.watch("same", "/current", false).unwrap();
    assert!(files.files.active.lock().unwrap().contains(&id));
    drop(second);
}

#[test]
fn test_監視_repositoryの再走査競合と下位エラーの分類を保持する() {
    use crate::usecase::repository_state::RepositoryStateError;

    // Given
    for source in [
        RepositoryStateError::ScanInvalidated,
        RepositoryStateError::Repository(
            crate::domain::repository::RepositoryError::rule("state").into(),
        ),
        RepositoryStateError::Code(
            crate::domain::code::CodeError::StaleReviewBlobVersion {
                requested: 1,
                current: 2,
            }
            .into(),
        ),
    ] {
        let expected = format!("{source:?}");
        let message = source.to_string();
        // When
        let error = UsecaseError::Repository(source);
        // Then
        assert!(
            matches!(&error, UsecaseError::Repository(actual) if format!("{actual:?}") == expected)
        );
        assert_eq!(error.to_string(), message);
    }
}

#[tokio::test]
async fn test_監視登録破棄_通常停止が失敗しても資源と購読の枠を解放する() {
    // Given
    let files = Arc::new(SubscriptionFiles::default());
    files
        .fail_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let usecase = Arc::new(WatcherUsecase::new(None, files.clone()));
    let subscription = usecase.subscribe("push".into()).unwrap();
    // When / Then
    for _ in 0..65 {
        let id = usecase.watch("push", "/file", false).unwrap();
        assert!(usecase.stop(id).is_err());
        usecase.release(id);
        usecase.release(id);
        assert!(files.active.lock().unwrap().is_empty());
    }
    drop(subscription);
}
