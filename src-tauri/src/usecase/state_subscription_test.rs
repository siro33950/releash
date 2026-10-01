use super::*;

#[derive(Default)]
struct RecordingOutput {
    initial: Mutex<Vec<SubscriptionTarget>>,
    failures: Mutex<Vec<(SubscriptionTarget, String)>>,
    starts: Mutex<Vec<SubscriptionTarget>>,
    cursors: Mutex<Vec<Option<(String, u64)>>>,
    stops: Mutex<Vec<SubscriptionTarget>>,
    fail_start: std::sync::atomic::AtomicBool,
    fail_initial: std::sync::atomic::AtomicBool,
    updates: Mutex<Vec<SubscriptionTarget>>,
    updated: tokio::sync::Notify,
}

impl StateSubscriptionOutput for RecordingOutput {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        _: &str,
        target: &SubscriptionTarget,
        cursor: Option<(&str, u64)>,
    ) -> Result<(), StateReadError> {
        if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StateReadError::from_error(SubscriptionError::UnknownTarget));
        }
        self.starts.lock().push(target.clone());
        self.cursors
            .lock()
            .push(cursor.map(|(epoch, sequence)| (epoch.into(), sequence)));
        Ok(())
    }

    fn stop(
        &self,
        _: &str,
        target: &SubscriptionTarget,
        _: &std::collections::HashSet<SubscriptionTarget>,
    ) -> Result<(), SubscriptionError> {
        self.stops.lock().push(target.clone());
        Ok(())
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        self.failures
            .lock()
            .push((target.clone(), error.to_string()));
        self.updated.notify_one();
        Ok(())
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        _: StateValue,
    ) -> Result<(), SubscriptionError> {
        if self.fail_initial.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SubscriptionError::EncodingFailed);
        }
        self.initial.lock().push(target.clone());
        Ok(())
    }

    fn publish(
        &self,
        target: &SubscriptionTarget,
        _: StateValue,
        _: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.updates.lock().push(target.clone());
        self.updated.notify_one();
        Ok(())
    }
}

struct PendingTimer;

impl SubscriptionTimer for PendingTimer {
    fn interval(&self, _: std::time::Duration) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>> {
        Box::pin(futures_util::stream::pending())
    }
}

#[tokio::test]
async fn test_購読手順_開始と停止で購読状態と出力を更新する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(
            Arc::new(RecordingReads {
                calls: Default::default(),
            }),
            None,
            vec![],
            String::new(),
        );
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    // When
    usecase
        .start_subscription("client", &target, Some(("prior", 4)))
        .await
        .unwrap();
    assert!(usecase.active_targets().contains(&target));
    usecase.stop_subscription("client", &target).await.unwrap();
    // Then
    assert!(usecase.active_targets().is_empty());
    assert_eq!(*output.starts.lock(), vec![target.clone()]);
    assert_eq!(*output.cursors.lock(), vec![Some(("prior".into(), 4))]);
    assert_eq!(*output.stops.lock(), vec![target]);
}

#[tokio::test]
async fn test_購読手順_配信側の開始失敗時にclientの対象を戻す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    output
        .fail_start
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let usecase = StateSubscriptionUsecase::new_with_output(output, Arc::new(PendingTimer))
        .with_reads(
            Arc::new(RecordingReads {
                calls: Default::default(),
            }),
            None,
            vec![],
            String::new(),
        );
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    // When
    let result = usecase.start_subscription("client", &target, None).await;
    // Then
    assert!(matches!(result, Err(StateReadError {
        source: StateReadFailure::Subscription(error), ..
    }) if *error == SubscriptionError::UnknownTarget));
    assert!(usecase.active_targets().is_empty());
}

#[tokio::test]
async fn test_購読手順_streamが無いと開始できない() {
    // Given
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        Arc::new(PendingTimer),
    )
    .with_reads(
        Arc::new(RecordingReads {
            calls: Default::default(),
        }),
        None,
        vec![],
        String::new(),
    );
    let target = SubscriptionTarget::RepositoryPaths;
    // When
    let result = usecase.start_subscription("client", &target, None).await;
    // Then
    assert!(matches!(
        result,
        Err(StateReadError {
            source: StateReadFailure::Subscription(error),
            ..
        }) if *error == SubscriptionError::StreamEnded
    ));
    assert!(usecase.active_targets().is_empty());
}

#[test]
fn test_購読手順_対象を検証してclient状態を更新する() {
    let output = Arc::new(RecordingOutput::default());
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer));

    usecase.open_client("client".into()).unwrap();

    assert_eq!(
        usecase.start("missing", &SubscriptionTarget::RepositoryPaths),
        Err(SubscriptionError::StreamEnded)
    );
    usecase
        .start("client", &SubscriptionTarget::RepositoryPaths)
        .unwrap();
    assert!(usecase
        .active_targets()
        .contains(&SubscriptionTarget::RepositoryPaths));
    usecase
        .stop("client", &SubscriptionTarget::RepositoryPaths)
        .unwrap();

    assert!(usecase.active_targets().is_empty());
}

struct RecordingReads {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl StateSubscriptionRead for RecordingReads {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(match target {
            SubscriptionTarget::RepositoryPaths => {
                StateValue::RepositoryPaths(vec!["/repo".into()])
            }
            _ => StateValue::SessionNode(Some("node".into())),
        })
    }

    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
async fn test_購読手順_初回読取を共有し変化で再読取して最後の停止でworkerを解放する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RecordingReads {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::SessionNode("/repo".into(), "node".into());
    usecase.open_client("first".into()).unwrap();
    usecase.open_client("second".into()).unwrap();

    // When
    usecase.start_read("first", &target).await.unwrap();
    usecase.start_read("second", &target).await.unwrap();
    // Then
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(*output.initial.lock(), vec![target.clone()]);

    // When
    usecase.notify(StateChangeSource::Worktree("/repo".into()));
    tokio::time::timeout(std::time::Duration::from_secs(1), output.updated.notified())
        .await
        .unwrap();
    // Then
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(*output.updates.lock(), vec![target.clone()]);

    // When
    usecase.stop_read("first", &target).await.unwrap();
    // Then
    assert_eq!(usecase.test_worker_count(), 1);
    // When
    usecase.stop_read("second", &target).await.unwrap();
    // Then
    assert_eq!(usecase.test_worker_count(), 0);
}

#[tokio::test]
async fn test_購読手順_任意の対象で配信完了を待ち一度だけ読み直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RecordingReads {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::SessionNode("/repo".into(), "node".into());
    usecase.open_client("client".into()).unwrap();
    usecase.start_read("client", &target).await.unwrap();
    // When
    let wait_target = target.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking(move || {
            usecase.notify_and_wait(StateChangeSource::Worktree("/repo".into()), &wait_target);
        }),
    )
    .await
    .unwrap()
    .unwrap();
    // Then
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_配信完了待機_待機対象以外の対象にも同じ変化を配信する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RecordingReads {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let waited = SubscriptionTarget::SessionNode("/repo".into(), "one".into());
    let other = SubscriptionTarget::SessionNode("/repo".into(), "two".into());
    usecase.open_client("client".into()).unwrap();
    usecase.start_read("client", &waited).await.unwrap();
    usecase.start_read("client", &other).await.unwrap();

    // When
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking({
            let usecase = usecase.clone();
            let waited = waited.clone();
            move || usecase.notify_and_wait(StateChangeSource::Worktree("/repo".into()), &waited)
        }),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if output.updates.lock().len() == 2 {
                break;
            }
            output.updated.notified().await;
        }
    })
    .await
    .unwrap();

    // Then
    let updates = output.updates.lock();
    assert_eq!(
        updates.iter().filter(|target| **target == waited).count(),
        1
    );
    assert_eq!(updates.iter().filter(|target| **target == other).count(), 1);
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), 4);
}

/// 2 回目の読み取りを `release` まで止め、読み取りと外部の取り直しの回数を数える。
#[derive(Default)]
struct GatedReads {
    reads: std::sync::atomic::AtomicUsize,
    external: std::sync::atomic::AtomicUsize,
    blocked: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl GatedReads {
    fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn external(&self) -> usize {
        self.external.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StateSubscriptionRead for GatedReads {
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        if self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            self.blocked.notify_one();
            self.release.notified().await;
        }
        Ok(StateValue::SessionNode(None))
    }

    async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
        self.external
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

/// 読み取りを止めている間に `changes` を知らせ、その後の配信が落ち着くまで待つ。
async fn notify_while_reading(
    target: SubscriptionTarget,
    first: StateChangeSource,
    changes: Vec<StateChangeSource>,
) -> (Arc<GatedReads>, Arc<RecordingOutput>) {
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    usecase.open_client("client".into()).unwrap();
    usecase.start_read("client", &target).await.unwrap();
    usecase.notify(first);
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    for change in changes {
        usecase.notify(change);
    }
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while output.updates.lock().len() < 2 {
            output.updated.notified().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (reads, output)
}

#[tokio::test]
async fn test_購読手順_読み取り中に溜まった知らせを一回の読み取りにまとめる() {
    // Given / When: 読み取りの間に同じ対象の変化が 5 回届く
    let target = SubscriptionTarget::Workspaces;
    let (reads, output) = notify_while_reading(
        target.clone(),
        StateChangeSource::Worktree("/repo".into()),
        vec![StateChangeSource::Worktree("/repo".into()); 5],
    )
    .await;

    // Then: 初回・止めていた読み取り・まとめた 1 回だけ読み、外部の情報は取り直さない
    assert_eq!(reads.reads(), 3);
    assert_eq!(*output.updates.lock(), vec![target.clone(), target]);
    assert_eq!(reads.external(), 1);
}

#[tokio::test]
async fn test_購読手順_知らせを取りこぼしても読み直すだけで外部の情報は取り直さない() {
    // Given / When: 読み取りの間に、知らせの保持数を超える変化が届く
    let target = SubscriptionTarget::Workspaces;
    let (reads, output) = notify_while_reading(
        target.clone(),
        StateChangeSource::Worktree("/repo".into()),
        vec![StateChangeSource::Worktree("/repo".into()); 200],
    )
    .await;

    // Then
    assert_eq!(reads.reads(), 3);
    assert_eq!(output.updates.lock().len(), 2);
    assert_eq!(reads.external(), 1);
}

#[tokio::test]
async fn test_購読手順_repositoryの増減がまとめた知らせにあれば外部の情報を取り直す() {
    // Given / When: 読み取りの間に、Repository の増減とほかの変化が届く
    let (reads, output) = notify_while_reading(
        SubscriptionTarget::Workspaces,
        StateChangeSource::Worktree("/repo".into()),
        vec![
            StateChangeSource::Repositories,
            StateChangeSource::Worktree("/repo".into()),
        ],
    )
    .await;

    // Then: 開始時の 1 回に加えて、もう 1 回だけ取り直す
    assert_eq!(reads.reads(), 3);
    assert_eq!(output.updates.lock().len(), 2);
    assert_eq!(reads.external(), 2);
}

struct FailingReads {
    fail_read: std::sync::atomic::AtomicBool,
    fail_refresh: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl StateSubscriptionRead for FailingReads {
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        if self.fail_read.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StateReadError::from_error(SubscriptionError::UnknownTarget));
        }
        Ok(StateValue::RepositoryPaths(vec!["/repo".into()]))
    }
    async fn refresh_external(&self, _: &SubscriptionTarget) -> Result<(), StateReadError> {
        if self.fail_refresh.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StateReadError::from_error(
                SubscriptionError::EncodingFailed,
            ));
        }
        Ok(())
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}
#[tokio::test]
async fn test_購読読取_初回失敗後も登録を残す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(FailingReads {
        fail_read: true.into(),
        fail_refresh: false.into(),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    // When
    usecase
        .start_subscription("client", &target, None)
        .await
        .unwrap();
    // Then
    assert!(usecase.active_targets().contains(&target));
    assert_eq!(output.failures.lock().len(), 1);
}
#[tokio::test]
async fn test_購読読取_初回失敗から回復した値を配信する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(FailingReads {
        fail_read: true.into(),
        fail_refresh: false.into(),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, None)
        .await
        .unwrap();
    // When
    reads
        .fail_read
        .store(false, std::sync::atomic::Ordering::SeqCst);
    output.updated.notified().await;
    usecase.notify(StateChangeSource::Repositories);
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    usecase.close_client("client");
    // Then
    assert_eq!(output.updates.lock().len(), 1);
}
#[tokio::test]
async fn test_購読読取_回復後の再失敗を配信する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(FailingReads {
        fail_read: true.into(),
        fail_refresh: false.into(),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, None)
        .await
        .unwrap();
    reads
        .fail_read
        .store(false, std::sync::atomic::Ordering::SeqCst);
    output.updated.notified().await;
    usecase.notify(StateChangeSource::Repositories);
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    // When
    reads
        .fail_read
        .store(true, std::sync::atomic::Ordering::SeqCst);
    usecase.notify(StateChangeSource::Repositories);
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    usecase.close_client("client");
    // Then
    assert_eq!(output.failures.lock().len(), 2);
}
struct RefreshTimer(Arc<tokio::sync::Notify>);
impl SubscriptionTimer for RefreshTimer {
    fn interval(&self, _: std::time::Duration) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>> {
        Box::pin(futures_util::stream::unfold(
            self.0.clone(),
            |notify| async move {
                notify.notified().await;
                Some(((), notify))
            },
        ))
    }
}
#[tokio::test]
async fn test_購読外部読取_再取得失敗で古いキャッシュを配信しない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(FailingReads {
        fail_read: false.into(),
        fail_refresh: false.into(),
    });
    let tick = Arc::new(tokio::sync::Notify::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        Arc::new(RefreshTimer(tick.clone())),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::Issues("/repo".into());
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, None)
        .await
        .unwrap();
    // When
    reads
        .fail_refresh
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    // Then
    assert_eq!(output.failures.lock().len(), 1);
    assert!(output.updates.lock().is_empty());
    usecase.close_client("client");
}

fn notion_target() -> SubscriptionTarget {
    SubscriptionTarget::NotionTasks("/repo".into(), 20, None, Default::default())
}

#[tokio::test]
async fn test_notion購読_同じ対象の2つ目の開始では取り直さない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let subscriptions =
        StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
            .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions.start_read("a", &target).await.unwrap();
    // When
    subscriptions.start_read("b", &target).await.unwrap();
    subscriptions.close_client("a");
    let worker_count = subscriptions.test_worker_count();
    subscriptions.close_client("b");
    // Then
    assert_eq!(reads.external(), 1);
    assert_eq!(reads.reads(), 1);
    assert_eq!(*output.initial.lock(), vec![target]);
    assert_eq!(worker_count, 1);
}

#[tokio::test]
async fn test_notion購読_対象repoの設定変更で取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let subscriptions =
        StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
            .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.start_read("a", &target).await.unwrap();
    // When
    subscriptions.notify(StateChangeSource::NotionConfig("/repo".into()));
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("a");
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_notion購読_repositoryの増減で取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let subscriptions =
        StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
            .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.start_read("a", &target).await.unwrap();
    // When
    subscriptions.notify(StateChangeSource::Repositories);
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("a");
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_notion購読_最後のclientが閉じたらworkerを止める() {
    // Given
    let reads = Arc::new(GatedReads::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        Arc::new(PendingTimer),
    )
    .with_reads(reads, None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions.start_read("a", &target).await.unwrap();
    subscriptions.start_read("b", &target).await.unwrap();
    subscriptions.close_client("a");
    let before = subscriptions.test_worker_count();
    // When
    subscriptions.close_client("b");
    // Then
    assert_eq!(before, 1);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_notion購読_タスクの一覧を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        Arc::new(RefreshTimer(tick.clone())),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("client".into()).unwrap();
    subscriptions.start_read("client", &target).await.unwrap();
    // When
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("client");
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_notion購読_ラベルの選択肢を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        Arc::new(RefreshTimer(tick.clone())),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::NotionLabelOptions("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    subscriptions.start_read("client", &target).await.unwrap();
    // When
    tick.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.blocked.notified())
        .await
        .unwrap();
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), output.updated.notified())
        .await
        .unwrap();
    subscriptions.close_client("client");
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[derive(Default)]
struct RetainingReads {
    retained: Mutex<std::collections::HashSet<SubscriptionTarget>>,
    releases: Mutex<Vec<SubscriptionTarget>>,
    pause: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl StateSubscriptionRead for RetainingReads {
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        if self.pause.load(std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.retained.lock().insert(target.clone());
        Ok(())
    }
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        Ok(StateValue::SessionNode(None))
    }
    fn release_external(&self, target: &SubscriptionTarget) {
        self.retained.lock().remove(target);
        self.releases.lock().push(target.clone());
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
    fn review_comments_dir(&self) -> String {
        "/missing".into()
    }
}

#[tokio::test]
async fn test_notion購読_初回取得中にclientが閉じたら開始の対象だけ解放する() {
    // Given
    let reads = Arc::new(RetainingReads::default());
    reads.pause.store(true, std::sync::atomic::Ordering::SeqCst);
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        Arc::new(PendingTimer),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    let other = SubscriptionTarget::NotionLabelOptions("/other".into());
    reads.retained.lock().insert(other.clone());
    subscriptions.open_client("client".into()).unwrap();
    let starting = subscriptions.clone();
    let start_target = target.clone();
    // When
    let task = tokio::spawn(async move { starting.start_read("client", &start_target).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.entered.notified())
        .await
        .unwrap();
    subscriptions.close_client("client");
    reads.resume.notify_one();
    let result = task.await.unwrap();
    // Then
    assert!(
        matches!(result, Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::StreamEnded)
    );
    assert!(!reads.retained.lock().contains(&target));
    assert!(reads.retained.lock().contains(&other));
    assert_eq!(*reads.releases.lock(), vec![target]);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_notion購読_監視の更新が失敗したら開始の対象だけ解放する() {
    // Given
    let reads = Arc::new(RetainingReads::default());
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default()),
    ));
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        Arc::new(PendingTimer),
    )
    .with_reads(reads.clone(), Some(watcher), vec![], String::new());
    let target = notion_target();
    let other = SubscriptionTarget::NotionLabelOptions("/other".into());
    reads.retained.lock().insert(other.clone());
    subscriptions.open_client("client".into()).unwrap();
    subscriptions.open_client("watcher".into()).unwrap();
    subscriptions
        .start("watcher", &SubscriptionTarget::ReviewThreads("repo".into()))
        .unwrap();
    // When
    let result = subscriptions.start_read("client", &target).await;
    subscriptions.close_client("watcher");
    subscriptions.close_client("client");
    // Then
    assert!(matches!(
        result,
        Err(StateReadError {
            source: StateReadFailure::Watcher(_),
            ..
        })
    ));
    assert!(!reads.retained.lock().contains(&target));
    assert!(reads.retained.lock().contains(&other));
    assert_eq!(*reads.releases.lock(), vec![target]);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_notion購読_最初の値の配信が失敗したら開始の対象だけ解放する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    output
        .fail_initial
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let reads = Arc::new(RetainingReads::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(output, Arc::new(PendingTimer))
        .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    let other = SubscriptionTarget::NotionLabelOptions("/other".into());
    reads.retained.lock().insert(other.clone());
    subscriptions.open_client("client".into()).unwrap();
    // When
    let result = subscriptions.start_read("client", &target).await;
    subscriptions.close_client("client");
    // Then
    assert!(
        matches!(result, Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::EncodingFailed)
    );
    assert!(!reads.retained.lock().contains(&target));
    assert!(reads.retained.lock().contains(&other));
    assert_eq!(*reads.releases.lock(), vec![target]);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_notion購読_別のclientの開始が失敗しても購読中の対象は解放しない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RetainingReads::default());
    let subscriptions =
        StateSubscriptionUsecase::new_with_output(output.clone(), Arc::new(PendingTimer))
            .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions
        .start_subscription("a", &target, None, None)
        .await
        .unwrap();
    output
        .fail_start
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // When
    let result = subscriptions
        .start_subscription("b", &target, None, None)
        .await;
    let retained = reads.retained.lock().contains(&target);
    let released = reads.releases.lock().clone();
    let workers = subscriptions.test_worker_count();
    subscriptions.close_client("a");
    subscriptions.close_client("b");
    // Then
    assert!(result.is_err());
    assert!(retained);
    assert!(released.is_empty());
    assert_eq!(workers, 1);
}
