use super::*;

#[derive(Default)]
struct RecordingOutput {
    initial: Mutex<Vec<SubscriptionTarget>>,
    starts: Mutex<Vec<SubscriptionTarget>>,
    cursors: Mutex<Vec<Option<(String, u64)>>>,
    stops: Mutex<Vec<SubscriptionTarget>>,
    fail_start: std::sync::atomic::AtomicBool,
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
        _: Option<&str>,
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

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        _: StateValue,
    ) -> Result<(), SubscriptionError> {
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

    fn set_terminal_snapshot(
        &self,
        _: &SubscriptionTarget,
        _: u64,
        _: u64,
        _: StateValue,
    ) -> Result<(), SubscriptionError> {
        unreachable!()
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
        .start_subscription("client", &target, None, Some(("prior", 4)))
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
    let result = usecase
        .start_subscription("client", &target, None, None)
        .await;
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
    let result = usecase
        .start_subscription("client", &target, None, None)
        .await;
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

    async fn refresh_workspaces(&self, _: Option<StateChangeSource>) {}

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
