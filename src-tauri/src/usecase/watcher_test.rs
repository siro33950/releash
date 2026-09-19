use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Files(Mutex<Vec<String>>);
impl FileWatchGateway for Files {
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
    assert_eq!(usecase.start("/file"), Ok(42));
    assert_eq!(usecase.stop(42), Ok(()));
    assert_eq!(
        usecase.start("/missing"),
        Err(UsecaseError::File("missing path".into()))
    );
    assert_eq!(
        usecase.stop(999),
        Err(UsecaseError::File("unknown watcher".into()))
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
    assert_eq!(
        usecase.stop(u64::MAX),
        Err(UsecaseError::File("unknown watcher".into()))
    );
}

#[derive(Default)]
pub(crate) struct SubscriptionFiles {
    pub(crate) next: std::sync::atomic::AtomicU64,
    pub(crate) active: Mutex<std::collections::HashSet<u64>>,
    pub(crate) fail_stop: std::sync::atomic::AtomicBool,
}
impl FileWatchGateway for SubscriptionFiles {
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
    assert_eq!(
        usecase.stop(id),
        Err(UsecaseError::File("stop failed".into()))
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
    drop(first);
    let second = usecase.subscribe("same".into()).unwrap();
    release.send(()).unwrap();
    // Then
    assert_eq!(
        watching.await.unwrap(),
        Err(UsecaseError::Subscription(WatchSubscriptionError::NotFound))
    );
    assert!(files.files.active.lock().unwrap().is_empty());
    let id = usecase.watch("same", "/current", false).unwrap();
    assert!(files.files.active.lock().unwrap().contains(&id));
    drop(second);
}
