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

    fn invalidate(&self, _: StateChangeSource) {
        unreachable!()
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        tokio::sync::broadcast::channel(1).0,
        Arc::new(PendingTimer),
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output,
        tokio::sync::broadcast::channel(1).0,
        Arc::new(PendingTimer),
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
        tokio::sync::broadcast::channel(1).0,
        Arc::new(PendingTimer),
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        tokio::sync::broadcast::channel(1).0,
        Arc::new(PendingTimer),
    );

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
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(StateValue::SessionNode(Some("node".into())))
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
    let changes = tokio::sync::broadcast::channel(8).0;
    let reads = Arc::new(RecordingReads {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        changes.clone(),
        Arc::new(PendingTimer),
    )
    .with_reads(reads.clone(), None, vec![]);
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
    changes
        .send(StateChangeSource::Worktree("/repo".into()))
        .unwrap();
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

struct TestTimer;

impl SubscriptionTimer for TestTimer {
    fn interval(
        &self,
        duration: std::time::Duration,
    ) -> std::pin::Pin<Box<dyn Stream<Item = ()> + Send>> {
        let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + duration, duration);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Box::pin(futures_util::stream::unfold(
            timer,
            |mut timer| async move {
                timer.tick().await;
                Some(((), timer))
            },
        ))
    }
}

use crate::test_support::state_subscription::{
    same, start, start_read, stop, stop_read, StateReadsFixture,
};
use crate::test_support::state_subscription::{Delivery, Event, StateSubscriptionEvent, Version};
use crate::usecase::state_subscription::{StateReadError, StateReadFailure, StateSubscriptionRead};
const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test(start_paused = true)]
async fn test_購読_配信と定期印と終了時の解放() {
    // Given
    let usecase = StateSubscriptionUsecase::new(vec!["/repo".into()], Arc::new(TestTimer));
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    // When
    start(
        &usecase,
        "client",
        &SubscriptionTarget::RepositoryPaths.to_string(),
        None,
    )
    .unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::RepositoryPaths(vec!["/repo".into()])))
    );
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    usecase
        .publisher()
        .publish(
            &SubscriptionTarget::RepositoryPaths,
            StateValue::RepositoryPaths(vec!["/next".into()]),
            None,
        )
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(Version { sequence: 1, .. }, Delivery::Full, _)
        ))
    ));
    tokio::time::advance(BOOKMARK_INTERVAL).await;
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Bookmark(Version { sequence: 1, .. })
        ))
    ));
    drop(stream);
    assert_eq!(
        start(
            &usecase,
            "client",
            &SubscriptionTarget::RepositoryPaths.to_string(),
            None
        ),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(usecase.open("client".into()).is_ok());
}

#[tokio::test(start_paused = true)]
async fn test_購読_開始と配信と停止が待機中streamを起こす() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    struct WakeFlag(AtomicBool);
    impl Wake for WakeFlag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    // Given
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer));
    let mut stream = Box::pin(usecase.open("waiting".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    flag.0.store(false, Ordering::SeqCst);

    assert_eq!(
        start(&usecase, "waiting", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    assert_eq!(
        usecase.publisher().publish(
            &SubscriptionTarget::BranchBase("/missing".into(), "branch".into()),
            StateValue::RepositoryPaths(vec![]),
            None
        ),
        Err(SubscriptionError::UnknownTarget)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    start(
        &usecase,
        "waiting",
        &SubscriptionTarget::RepositoryPaths.to_string(),
        None,
    )
    .unwrap();

    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _))))
    ));
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_))))
    ));
    while let Poll::Ready(Some(event)) = stream.as_mut().poll_next(&mut cx) {
        assert!(matches!(
            event,
            StateSubscriptionEvent::Item(_, Event::Bookmark(_))
        ));
    }
    flag.0.store(false, Ordering::SeqCst);

    // When
    usecase
        .publisher()
        .publish(
            &SubscriptionTarget::RepositoryPaths,
            StateValue::RepositoryPaths(vec!["/next".into()]),
            None,
        )
        .unwrap();

    // Then
    assert!(flag.0.load(Ordering::SeqCst));
    assert!(
        matches!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Change(Version { sequence: 1, .. }, Delivery::Full, value)))) if same(&value, StateValue::RepositoryPaths(vec!["/next".into()])))
    );

    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    flag.0.store(false, Ordering::SeqCst);
    assert_eq!(
        stop(
            &usecase,
            "missing",
            &SubscriptionTarget::RepositoryPaths.to_string()
        ),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    stop(
        &usecase,
        "waiting",
        &SubscriptionTarget::RepositoryPaths.to_string(),
    )
    .unwrap();

    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    usecase
        .publisher()
        .publish(
            &SubscriptionTarget::RepositoryPaths,
            StateValue::RepositoryPaths(vec![]),
            None,
        )
        .unwrap();
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    tokio::time::advance(BOOKMARK_INTERVAL).await;
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
}

struct FakeReads {
    value: Mutex<String>,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl StateSubscriptionRead for FakeReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(match target {
            crate::usecase::state_subscription::SubscriptionTarget::SessionNode(_, _) => {
                StateValue::SessionNode(Some(self.value.lock().clone()))
            }
            crate::usecase::state_subscription::SubscriptionTarget::SessionHistory(_, _) => {
                StateValue::SessionHistory(
                    crate::usecase::agent_session::AgentSessionHistoryPageDto {
                        items: vec![],
                        has_more: false,
                    },
                )
            }
            _ => StateValue::Issues(vec![]),
        })
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::usecase::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
async fn test_引数付き購読_対象の変更だけを読み直して配信し終了でworkerを解放する() {
    use crate::usecase::state_subscription::{StateChangeSource, SubscriptionTarget};
    // Given
    let reads = Arc::new(FakeReads {
        value: Mutex::new("node-1".into()),
        calls: Default::default(),
    });
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        reads.clone(),
        None,
        vec![],
    );
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::SessionNode("/repo".into(), "session".into()).to_string();
    // When
    start_read(&usecase, "client", &target, None).await.unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::SessionNode(Some("node-1".into()))))
    );
    stream.next().await;
    *reads.value.lock() = "node-2".into();
    usecase
        .publisher()
        .invalidate(StateChangeSource::Worktree("/repo".into()));
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::SessionNode(Some("node-2".into()))))
    );
    drop(stream);
    assert_eq!(usecase.test_worker_count(), 0);
    assert!(usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.active_targets().is_empty()));
}

#[tokio::test(start_paused = true)]
async fn test_外部情報_購読者がいる間だけcache_ttlで取得する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use std::sync::atomic::Ordering;
    // Given
    let reads = Arc::new(FakeReads {
        value: Mutex::new(String::new()),
        calls: Default::default(),
    });
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        reads.clone(),
        None,
        vec![],
    );
    let stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::Issues("/repo".into()).to_string();
    start_read(&usecase, "client", &target, None).await.unwrap();
    tokio::task::yield_now().await;
    let initial = reads.calls.load(Ordering::SeqCst);
    // When
    tokio::time::advance(crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration()).await;
    tokio::task::yield_now().await;
    // Then
    assert!(reads.calls.load(Ordering::SeqCst) > initial);
    drop(stream);
    let stopped = reads.calls.load(Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(reads.calls.load(Ordering::SeqCst), stopped);
}

#[tokio::test]
async fn test_履歴購読_件数違いと別clientが監視を共有し最後の終了で解放する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        Arc::new(FakeReads {
            value: Mutex::new(String::new()),
            calls: Default::default(),
        }),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec!["/claude".into(), "/codex".into()],
    );
    let first = usecase.open("first".into()).unwrap();
    let second = usecase.open("second".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
    start_read(&usecase, "first", &target, None).await.unwrap();
    start_read(&usecase, "second", &target, None).await.unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 2);
    assert_eq!(usecase.test_worker_count(), 1);
    stop(&usecase, "first", &target).unwrap();
    let expanded = SubscriptionTarget::SessionHistory("/repo".into(), 40).to_string();
    start_read(&usecase, "first", &expanded, None)
        .await
        .unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 2);
    drop(second);
    assert_eq!(files.active.lock().unwrap().len(), 2);
    drop(first);
    assert!(files.active.lock().unwrap().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
}

#[tokio::test]
async fn test_workspaces購読_最後の停止と切断で実際のgit監視を解放する() {
    use crate::usecase::state_subscription::WatchRequirement;
    for disconnect in [false, true] {
        // Given
        let fixture = StateReadsFixture::new();
        let repository = fixture.reads.repository_state.clone();
        let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
        let usecase = fixture.subscriptions.clone().with_reads(
            Arc::new(fixture.reads.clone()),
            Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                Some(repository.clone()),
                files.clone(),
            ))),
            vec![],
        );
        let first = usecase.open("first".into()).unwrap();
        let second = usecase.open("second".into()).unwrap();
        start_read(&usecase, "first", "workspaces", None)
            .await
            .unwrap();
        start_read(&usecase, "second", "workspaces", None)
            .await
            .unwrap();
        let requirement = WatchRequirement::Git(fixture.path.clone());
        let id = usecase.test_watches()[&requirement];
        let snapshot = repository.get_snapshot(&fixture.path).unwrap();
        assert_eq!(usecase.test_watches().len(), 1);
        // When / Then
        stop_read(&usecase, "first", "workspaces").await.unwrap();
        assert_eq!(usecase.test_watches()[&requirement], id);
        assert!(Arc::ptr_eq(
            &snapshot,
            &repository.get_snapshot(&fixture.path).unwrap()
        ));
        if !disconnect {
            stop_read(&usecase, "second", "workspaces").await.unwrap();
        }
        drop(second);
        assert!(usecase.test_watches().is_empty());
        assert_eq!(usecase.test_worker_count(), 0);
        assert!(!repository.stop_watching(id).unwrap());
        assert!(!Arc::ptr_eq(
            &snapshot,
            &repository.get_snapshot(&fixture.path).unwrap()
        ));
        assert!(files.active.lock().unwrap().is_empty());
        drop(first);
    }
}

#[tokio::test]
async fn test_監視開始失敗_購読を残さず次の開始で再度監視を試みる() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        Arc::new(FakeReads {
            value: Mutex::new(String::new()),
            calls: Default::default(),
        }),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec!["/missing".into()],
    );
    let _stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
    assert!(start_read(&usecase, "client", &target, None).await.is_err());
    assert!(usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.active_targets().is_empty()));
    let usecase = usecase.with_reads(
        Arc::new(FakeReads {
            value: Mutex::new(String::new()),
            calls: Default::default(),
        }),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec!["/history".into()],
    );
    start_read(&usecase, "client", &target, None).await.unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 1);
}

struct BlockedReads {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl StateSubscriptionRead for BlockedReads {
    async fn read(
        &self,
        _: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(StateValue::SessionHistory(
            crate::usecase::agent_session::AgentSessionHistoryPageDto {
                items: vec![],
                has_more: false,
            },
        ))
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::usecase::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}
#[tokio::test]
async fn test_購読停止_初回読取中の停止要求でも監視とworkerを残さない() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    let reads = Arc::new(BlockedReads {
        entered: Default::default(),
        release: Default::default(),
    });
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        reads.clone(),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec!["/history".into()],
    );
    let _stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
    let started = tokio::spawn({
        let usecase = usecase.clone();
        let target = target.clone();
        async move { start_read(&usecase, "client", &target, None).await }
    });
    reads.entered.notified().await;
    let stopped = tokio::spawn({
        let usecase = usecase.clone();
        async move { stop_read(&usecase, "client", &target).await }
    });
    tokio::task::yield_now().await;
    assert!(!stopped.is_finished());
    reads.release.notify_one();
    started.await.unwrap().unwrap();
    stopped.await.unwrap().unwrap();
    assert!(files.active.lock().unwrap().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
    assert!(usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.active_targets().is_empty()));
}

struct NullableReads;
#[async_trait::async_trait]
impl StateSubscriptionRead for NullableReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        Ok(match target {
            crate::usecase::state_subscription::SubscriptionTarget::AgentSession(_) => {
                StateValue::AgentSession(None)
            }
            crate::usecase::state_subscription::SubscriptionTarget::NodeDetail(_, _) => {
                StateValue::NodeDetail(None)
            }
            _ => panic!("unexpected target"),
        })
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::usecase::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
async fn test_不在対象_初回からnullable_snapshotとして配信する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        Arc::new(NullableReads),
        None,
        vec![],
    );
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    stream.next().await;
    for (target, expected) in [
        (
            SubscriptionTarget::AgentSession("missing".into()),
            StateValue::AgentSession(None),
        ),
        (
            SubscriptionTarget::NodeDetail("/repo".into(), "missing".into()),
            StateValue::NodeDetail(None),
        ),
    ] {
        start_read(&usecase, "client", &target.to_string(), None)
            .await
            .unwrap();
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, expected))
        );
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_購読開始と切断_同じ対象の最終購読者が切断しても再開できる() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        Arc::new(NullableReads),
        None,
        vec![],
    );
    let target = SubscriptionTarget::AgentSession("missing".into()).to_string();
    for index in 0..100 {
        let old_id = format!("old-{index}");
        let next_id = format!("next-{index}");
        let old = usecase.open(old_id.clone()).unwrap();
        start_read(&usecase, &old_id, &target, None).await.unwrap();
        let mut next = Box::pin(usecase.open(next_id.clone()).unwrap());
        next.next().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let closed = tokio::spawn({
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                drop(old);
            }
        });
        barrier.wait().await;
        start_read(&usecase, &next_id, &target, None).await.unwrap();
        closed.await.unwrap();
        assert!(
            matches!(next.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::AgentSession(None)))
        );
        drop(next);
        assert_eq!(usecase.test_worker_count(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn test_初回読取_保持中の版から再開して変更だけ届ける() {
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        Arc::new(FakeReads {
            value: Mutex::new("after".into()),
            calls: Default::default(),
        }),
        None,
        vec![],
    );
    let target = SubscriptionTarget::SessionNode("/repo".into(), "node".into()).to_string();
    let presenter = usecase.test_presenter().unwrap();
    let before = crate::test_support::state_subscription::payload(&StateValue::SessionNode(Some(
        "before".into(),
    )))
    .unwrap();
    let after = crate::test_support::state_subscription::payload(&StateValue::SessionNode(Some(
        "after".into(),
    )))
    .unwrap();
    let version = presenter.test_runtime().mutate(|state| {
        state
            .register(target.clone(), before, Delivery::Full)
            .unwrap();
        let version = state.current_version(&target).unwrap();
        state.publish(&target, after, None).unwrap();
        (version, true)
    });
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));

    tokio::time::advance(BOOKMARK_INTERVAL - std::time::Duration::from_millis(1)).await;

    start_read(
        &usecase,
        "client",
        &target,
        Some((&version.epoch, version.sequence)),
    )
    .await
    .unwrap();

    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Change(next, Delivery::Full, value)))
            if next.sequence == version.sequence + 1
                && same(&value, StateValue::SessionNode(Some("after".into())))
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(next)))
            if next.sequence == version.sequence + 1
    ));
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(next)))
            if next.sequence == version.sequence + 1
    ));
}

#[derive(Default)]
struct ExternalReads {
    issues: std::sync::atomic::AtomicU64,
    prs: std::sync::atomic::AtomicU64,
}
impl ExternalReads {
    fn value(&self, target: &crate::usecase::state_subscription::SubscriptionTarget) -> StateValue {
        use crate::usecase::workspace_tree::{
            WorkspaceBranchDto, WorkspaceListSnapshotDto, WorkspaceListStatusDto,
            WorkspaceRepositoryListDto,
        };
        use std::sync::atomic::Ordering;
        if matches!(
            target,
            crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
        ) {
            return StateValue::Issues(vec![crate::usecase::git_host::IssueInfoDto {
                number: self.issues.load(Ordering::SeqCst),
                default_branch_name: "main".into(),
                title: "issue".into(),
                state: "open".into(),
                url: String::new(),
                author: crate::usecase::git_host::dto::PrAuthorDto {
                    login: "author".into(),
                },
                created_at: String::new(),
                updated_at: String::new(),
                labels: vec![],
                assignees: vec![],
                body: String::new(),
                milestone: None,
            }]);
        }
        let status = WorkspaceListStatusDto {
            loaded: true,
            state: "ready",
            error: None,
        };
        StateValue::Workspaces(WorkspaceListSnapshotDto {
            generation: 0,
            status: status.clone(),
            repositories: vec![WorkspaceRepositoryListDto {
                path: "/repo".into(),
                status,
                worktrees: vec![],
                branches: vec![WorkspaceBranchDto {
                    branch: crate::usecase::repository_dto::BranchCardDto {
                        name: "main".into(),
                        is_deleting: false,
                        is_main_worktree: true,
                        worktree_path: Some("/repo".into()),
                        dirty_count: 0,
                        is_merged: false,
                        ahead: 0,
                        behind: 0,
                        has_upstream: false,
                        base_ahead: 0,
                    },
                    has_pr: true,
                    pr_number: Some(self.prs.load(Ordering::SeqCst)),
                    pr_url: None,
                }],
            }],
        })
    }
}
#[async_trait::async_trait]
impl StateSubscriptionRead for ExternalReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        Ok(self.value(target))
    }
    async fn refresh_external(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        if matches!(
            target,
            crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
        ) {
            self.issues
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::usecase::state_subscription::StateChangeSource>,
    ) {
        self.prs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn repositories(&self) -> Vec<String> {
        vec!["/repo".into()]
    }
}

#[tokio::test(start_paused = true)]
async fn test_外部情報ttl_issueとprを取得し更新値を配信して停止後は取得しない() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    use std::sync::atomic::Ordering;
    for target in [
        SubscriptionTarget::Issues("/repo".into()),
        SubscriptionTarget::Workspaces,
    ] {
        let reads = Arc::new(ExternalReads::default());
        let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
            reads.clone(),
            None,
            vec![],
        );
        let mut stream = Box::pin(usecase.open("client".into()).unwrap());
        stream.next().await;
        start_read(&usecase, "client", &target.to_string(), None)
            .await
            .unwrap();
        let count = if matches!(target, SubscriptionTarget::Issues(_)) {
            &reads.issues
        } else {
            &reads.prs
        };
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, reads.value(&target)))
        );
        stream.next().await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(29)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let changed = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(_, Delivery::Full, value),
                )) = stream.next().await
                {
                    break value;
                }
            }
        })
        .await
        .unwrap();
        assert!(same(&changed, reads.value(&target)));
        drop(stream);
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn test_初回読取中の切断_開始失敗後に対象の鍵もworkerも残さない() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    // Given
    let reads = Arc::new(BlockedReads {
        entered: Default::default(),
        release: Default::default(),
    });
    let usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer)).with_reads(
        reads.clone(),
        None,
        vec![],
    );
    let stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20);
    let started = tokio::spawn({
        let usecase = usecase.clone();
        let raw = target.to_string();
        async move { start_read(&usecase, "client", &raw, None).await }
    });
    reads.entered.notified().await;
    // When
    drop(stream);
    reads.release.notify_one();
    // Then
    let error = started.await.unwrap().unwrap_err();
    assert!(
        matches!(error.source, StateReadFailure::Subscription(source) if *source == SubscriptionError::StreamEnded)
    );
    assert_eq!(error.message, SubscriptionError::StreamEnded.to_string());
    assert!(!usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.registered(&target.to_string())));
    assert!(usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())));
    assert_eq!(usecase.test_worker_count(), 0);
}

struct DisconnectingFiles {
    usecase: StateSubscriptionUsecase,
}
impl crate::domain::repository::file_watcher::FileWatchGateway for DisconnectingFiles {
    fn release(&self, _: u64) {}
    fn start(&self, _: &str) -> Result<u64, String> {
        self.usecase.close_client("client");
        Ok(1)
    }
    fn stop(&self, _: u64) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn test_snapshot登録後の切断_開始失敗で対象の鍵を解放する() {
    use crate::usecase::state_subscription::SubscriptionTarget;
    // Given
    let mut usecase = StateSubscriptionUsecase::new(vec![], Arc::new(TestTimer));
    let files = Arc::new(DisconnectingFiles {
        usecase: usecase.clone(),
    });
    usecase = usecase.with_reads(
        Arc::new(FakeReads {
            value: Mutex::new(String::new()),
            calls: Default::default(),
        }),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None, files,
        ))),
        vec!["/history".into()],
    );
    let _stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20);
    // When
    let error = start_read(&usecase, "client", &target.to_string(), None)
        .await
        .unwrap_err();
    // Then
    assert_eq!(error.message, "StreamEnded");
    assert!(!usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.registered(&target.to_string())));
    assert!(usecase
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())));
    assert_eq!(usecase.test_worker_count(), 0);
}
