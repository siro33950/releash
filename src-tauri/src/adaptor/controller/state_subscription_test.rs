use crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter;
use crate::usecase::state_subscription::*;
use crate::usecase::test_helpers::state_subscription::{
    notion_target, FailingReads, GatedReads, RecordingOutput, RecordingReads,
};
use std::sync::Arc;
struct RecordingPresenter {
    recording: Arc<RecordingOutput>,
    presenter: Arc<StateSubscriptionPresenter>,
}

impl StateSubscriptionOutput for RecordingPresenter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn publish_failure(
        &self,
        target: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        self.recording
            .failures
            .lock()
            .push((target.clone(), error.to_string()));
        self.presenter.publish_failure(target, error)?;
        self.recording.updated.notify_one();
        Ok(())
    }

    fn publish_initial(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        self.presenter.publish_initial(target, snapshot.clone())?;
        self.recording.publish_initial(target, snapshot)
    }

    fn publish(
        &self,
        target: &SubscriptionTarget,
        snapshot: StateValue,
        delta: Option<StateValue>,
    ) -> Result<(), SubscriptionError> {
        self.presenter
            .publish(target, snapshot.clone(), delta.clone())?;
        self.recording.publish(target, snapshot, delta)
    }
}

#[tokio::test]
async fn test_購読外部読取_再取得失敗で古いキャッシュを配信しない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(FailingReads {
        fail_read: false.into(),
        fail_refresh: false.into(),
    });
    let tick = Arc::new(tokio::sync::Notify::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::Issues("/repo".into());
    usecase.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    usecase
        .start_subscription("client", &target, &delivery)
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
    presenter.close("client", &usecase.active_targets());
}

#[tokio::test]
async fn test_notion購読_タスクの一覧を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
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
    presenter.close("client", &subscriptions.active_targets());
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_notion購読_ラベルの選択肢を共通timerで取り直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(GatedReads::default());
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::NotionLabelOptions("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
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
    presenter.close("client", &subscriptions.active_targets());
    // Then
    assert_eq!(reads.external(), 2);
    assert_eq!(*output.updates.lock(), vec![target]);
}

#[tokio::test]
async fn test_購読駆動_時刻streamが終わっても変更通知で読み直す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
            Box::pin(futures_util::stream::empty())
        })),
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
    usecase.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    usecase
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    tokio::task::yield_now().await;
    // When
    usecase.notify(StateChangeSource::Repositories);
    output.updated.notified().await;
    // Then
    assert_eq!(*output.updates.lock(), vec![target]);
    usecase.close_client("client");
    presenter.close("client", &usecase.active_targets());
}

async fn assert_stopped(close: bool) {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let reads = Arc::new(RecordingReads {
        calls: Default::default(),
    });
    let tick = Arc::new(tokio::sync::Notify::new());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingPresenter {
            recording: output.clone(),
            presenter: presenter.clone(),
        }),
        super::drive(Arc::new({
            let tick = tick.clone();
            move || {
                Box::pin(futures_util::stream::unfold(
                    tick.clone(),
                    |tick| async move {
                        tick.notified().await;
                        Some(((), tick))
                    },
                ))
            }
        })),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::Issues("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    presenter.open("client".into()).unwrap();
    let delivery = presenter
        .reserve_delivery("client", "subscription", &target.to_string(), None)
        .unwrap();
    subscriptions
        .start_subscription("client", &target, &delivery)
        .await
        .unwrap();
    let calls = reads.calls.load(std::sync::atomic::Ordering::SeqCst);
    let snapshots = output.initial_values.lock().len() + output.update_values.lock().len();
    let failures = output.failures.lock().len();
    // When
    if close {
        subscriptions.close_client("client");
        presenter.close("client", &subscriptions.active_targets());
    } else {
        subscriptions
            .stop_subscription("client", &target, &delivery)
            .await
            .unwrap();
    }
    subscriptions.notify(StateChangeSource::Issues("/repo".into()));
    tick.notify_one();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // Then
    assert_eq!(reads.calls.load(std::sync::atomic::Ordering::SeqCst), calls);
    assert_eq!(
        output.initial_values.lock().len() + output.update_values.lock().len(),
        snapshots
    );
    assert_eq!(output.failures.lock().len(), failures);
    assert_eq!(subscriptions.test_worker_count(), 0);
}

#[tokio::test]
async fn test_購読駆動_最終購読者の停止後は通知と周期で読取も配信もしない() {
    assert_stopped(false).await;
}

#[tokio::test]
async fn test_購読駆動_最終clientの切断後は通知と周期で読取も配信もしない() {
    assert_stopped(true).await;
}

mod driver_tests {

    mod scenarios {
        use crate::usecase::state_subscription::*;
        use crate::usecase::test_helpers::{start_read, stop_read};
        use futures_util::{Stream, StreamExt};
        use parking_lot::Mutex;
        use std::sync::Arc;

        use crate::infrastructure::state_subscription::Version;
        use crate::test_support::state_subscription::WakeFlag;

        use crate::test_support::state_subscription::same;
        use crate::test_support::state_subscription::{Delivery, Event, StateSubscriptionEvent};
        use crate::usecase::state_subscription::{
            StateReadError, StateReadFailure, StateSubscriptionRead,
        };
        const BOOKMARK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

        #[tokio::test(start_paused = true)]
        async fn test_購読_配信と定期印と終了時の解放() {
            // Given
            let usecase = StateSubscriptionUsecase::new(
                vec!["/repo".into()],
                crate::test_support::state_subscription::read_driver(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            // When
            start_read(
                &usecase,
                "client",
                &SubscriptionTarget::RepositoryPaths.to_string(),
                None,
            )
            .await
            .unwrap();
            // Then
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::RepositoryPaths(vec!["/repo".into()])))
            );
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
            ));
            usecase.test_set_repository_paths(vec!["/next".into()]);
            usecase.notify(StateChangeSource::Repositories);
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
            assert!(
                matches!(start_read(&usecase, "client", &SubscriptionTarget::RepositoryPaths.to_string(), None).await,
        Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::StreamEnded)
            );
            assert!(usecase.open("client".into()).is_ok());
        }

        #[tokio::test(start_paused = true)]
        async fn test_購読_開始と配信と停止が待機中streamを起こす() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::task::{Context, Poll, Waker};

            // Given
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
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

            assert!(
                matches!(start_read(&usecase, "waiting", "missing", None).await,
        Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::UnknownTarget)
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
            start_read(
                &usecase,
                "waiting",
                &SubscriptionTarget::RepositoryPaths.to_string(),
                None,
            )
            .await
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
            usecase.test_set_repository_paths(vec!["/next".into()]);
            usecase.notify(StateChangeSource::Repositories);
            tokio::task::yield_now().await;

            // Then
            assert!(flag.0.load(Ordering::SeqCst));
            assert!(
                matches!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Change(Version { sequence: 1, .. }, Delivery::Full, value)))) if same(&value, StateValue::RepositoryPaths(vec!["/next".into()])))
            );

            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            flag.0.store(false, Ordering::SeqCst);
            assert_eq!(
                stop_read(
                    &usecase,
                    "missing",
                    &SubscriptionTarget::RepositoryPaths.to_string()
                )
                .await,
                Ok(())
            );
            assert!(!flag.0.load(Ordering::SeqCst));

            // When
            stop_read(
                &usecase,
                "waiting",
                &SubscriptionTarget::RepositoryPaths.to_string(),
            )
            .await
            .unwrap();

            // Then
            assert!(flag.0.swap(false, Ordering::SeqCst));
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            usecase.notify(StateChangeSource::Repositories);
            assert!(stream.as_mut().poll_next(&mut cx).is_pending());
            tokio::time::advance(BOOKMARK_INTERVAL).await;
            assert!(matches!(
                stream.as_mut().poll_next(&mut cx),
                Poll::Ready(Some(StateSubscriptionEvent::Bookmark))
            ));
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
                    crate::usecase::state_subscription::SubscriptionTarget::ReleashBase(_) => {
                        StateValue::ReleashBase(Some(self.value.lock().clone()))
                    }
                    crate::usecase::state_subscription::SubscriptionTarget::WorkflowSource(_) => {
                        StateValue::WorkflowSource(Some(self.value.lock().clone()))
                    }
                    crate::usecase::state_subscription::SubscriptionTarget::SessionHistory(
                        _,
                        _,
                    ) => StateValue::SessionHistory(
                        crate::usecase::agent_session::AgentSessionHistoryPageDto {
                            items: vec![],
                            has_more: false,
                        },
                    ),
                    _ => StateValue::Issues(crate::usecase::fetched::Fetched::ready(vec![])),
                })
            }
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
            fn workflows_dir(&self) -> String {
                "/workflows".into()
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
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = SubscriptionTarget::ReleashBase("/repo".into()).to_string();
            // When
            start_read(&usecase, "client", &target, None).await.unwrap();
            // Then
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::ReleashBase(Some("node-1".into()))))
            );
            stream.next().await;
            *reads.value.lock() = "node-2".into();
            usecase.notify(StateChangeSource::Repository(vec!["/repo".into()]));
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::ReleashBase(Some("node-2".into()))))
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
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
            let stream = usecase.open("client".into()).unwrap();
            let target = SubscriptionTarget::Issues("/repo".into()).to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            tokio::task::yield_now().await;
            let initial = reads.calls.load(Ordering::SeqCst);
            // When
            tokio::time::advance(
                crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration(),
            )
            .await;
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
            let files =
                Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default());
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/claude".into(), "/codex".into()],
                String::new(),
            );
            let first = usecase.open("first".into()).unwrap();
            let second = usecase.open("second".into()).unwrap();
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
            start_read(&usecase, "first", &target, None).await.unwrap();
            start_read(&usecase, "second", &target, None).await.unwrap();
            assert_eq!(files.active.lock().unwrap().len(), 2);
            assert_eq!(usecase.test_worker_count(), 1);
            stop_read(&usecase, "first", &target).await.unwrap();
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
        async fn test_automation購読_置き場の監視を共有し最後の終了で解放する() {
            use crate::usecase::state_subscription::{StateChangeSource, WatchRequirement};
            // Given
            let files =
                Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default());
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec![],
                String::new(),
            );
            let first = usecase.open("first".into()).unwrap();
            let second = usecase.open("second".into()).unwrap();
            // When
            start_read(&usecase, "first", "workflows", None)
                .await
                .unwrap();
            start_read(&usecase, "second", "diagnostics", None)
                .await
                .unwrap();
            // Then
            let requirement = WatchRequirement::Files(
                "/workflows".into(),
                StateChangeSource::WorkflowDefinitions,
            );
            assert_eq!(
                usecase.test_watches().keys().collect::<Vec<_>>(),
                vec![&requirement]
            );
            assert_eq!(files.active.lock().unwrap().len(), 1);
            assert_eq!(usecase.test_worker_count(), 2);
            drop(first);
            assert_eq!(files.active.lock().unwrap().len(), 1);
            assert_eq!(usecase.test_worker_count(), 1);
            drop(second);
            assert!(files.active.lock().unwrap().is_empty());
            assert!(usecase.test_watches().is_empty());
            assert_eq!(usecase.test_worker_count(), 0);
        }

        struct CapturingFiles {
            on_change: Mutex<Option<crate::domain::repository::file_watcher::WatchChangeHandler>>,
        }
        impl crate::domain::repository::file_watcher::FileWatchGateway for CapturingFiles {
            fn start_tree(
                &self,
                _: &str,
                on_change: crate::domain::repository::file_watcher::WatchChangeHandler,
            ) -> Result<u64, String> {
                *self.on_change.lock() = Some(on_change);
                Ok(1)
            }
            fn stop(&self, _: u64) -> Result<(), String> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn test_automation購読_置き場のファイル変化で読み直して配信する() {
            // Given
            let files = Arc::new(CapturingFiles {
                on_change: Mutex::new(None),
            });
            let reads = Arc::new(FakeReads {
                value: Mutex::new("first".into()),
                calls: Default::default(),
            });
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                reads.clone(),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec![],
                String::new(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = crate::usecase::state_subscription::SubscriptionTarget::WorkflowSource(
                "dev".into(),
            )
            .to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(..)))
            ));
            let on_change = files.on_change.lock().clone().unwrap();
            // When
            *reads.value.lock() = "second".into();
            on_change(Ok(()));
            // Then
            let value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if let Some(StateSubscriptionEvent::Item(id, Event::Change(_, _, value))) =
                        stream.next().await
                    {
                        assert_eq!(id, format!("client:{target}"));
                        break value;
                    }
                }
            })
            .await
            .unwrap();
            assert!(same(
                &value,
                &StateValue::WorkflowSource(Some("second".into()))
            ));
        }

        #[tokio::test]
        async fn test_監視開始失敗_失敗を購読へ届け再開で張り直す() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let files =
                Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default());
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new(String::new()),
                    calls: Default::default(),
                }),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/missing".into()],
                String::new(),
            );
            let mut stream = Box::pin(usecase.open("client".into()).unwrap());
            stream.next().await;
            let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
            start_read(&usecase, "client", &target, None).await.unwrap();
            assert!(matches!(stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value)))
            if matches!(value.as_ref(), crate::adaptor::presenter::state_subscription::PublishedState::Failure(error)
                if error.message.contains("missing path"))));
            assert!(usecase
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| !state.active_targets().is_empty()));
            stop_read(&usecase, "client", &target).await.unwrap();
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
                String::new(),
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
            let files =
                Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default());
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                reads.clone(),
                Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                    None,
                    files.clone(),
                ))),
                vec!["/history".into()],
                String::new(),
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
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
        }

        #[tokio::test]
        async fn test_不在対象_初回からnullable_snapshotとして配信する() {
            use crate::usecase::state_subscription::SubscriptionTarget;
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(Arc::new(NullableReads), None, vec![], String::new());
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
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(Arc::new(NullableReads), None, vec![], String::new());
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
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(
                Arc::new(FakeReads {
                    value: Mutex::new("after".into()),
                    calls: Default::default(),
                }),
                None,
                vec![],
                String::new(),
            );
            let target = SubscriptionTarget::ReleashBase("/repo".into()).to_string();
            let presenter = usecase.test_presenter().unwrap();
            let before = crate::test_support::state_subscription::payload(
                &StateValue::ReleashBase(Some("before".into())),
            )
            .unwrap();
            let after = crate::test_support::state_subscription::payload(&StateValue::ReleashBase(
                Some("after".into()),
            ))
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
                        && same(&value, StateValue::ReleashBase(Some("after".into())))
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
            fn value(
                &self,
                target: &crate::usecase::state_subscription::SubscriptionTarget,
            ) -> StateValue {
                use crate::usecase::fetched::Fetched;
                use crate::usecase::workspace_tree::{
                    WorkspaceList, WorkspaceListRepository, WorkspaceListWorktree,
                };
                use std::sync::atomic::Ordering;
                if matches!(
                    target,
                    crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
                ) {
                    return StateValue::Issues(Fetched::ready(vec![
                        crate::domain::git_host::IssueInfo {
                            number: self.issues.load(Ordering::SeqCst),
                            title: "issue".into(),
                            state: "open".into(),
                            url: String::new(),
                            author: crate::domain::git_host::PrAuthor {
                                login: "author".into(),
                            },
                            created_at: String::new(),
                            updated_at: String::new(),
                            labels: vec![],
                            assignees: vec![],
                            body: String::new(),
                            milestone: None,
                        },
                    ]));
                }
                StateValue::Workspaces(WorkspaceList {
                    repositories: vec![WorkspaceListRepository {
                        path: "/repo".into(),
                        worktrees: Fetched::ready(vec![WorkspaceListWorktree {
                            worktree: crate::domain::repository::Worktree {
                                name: "main".into(),
                                path: "/repo".into(),
                                branch: "main".into(),
                                is_main: true,
                                is_locked: false,
                                is_merged: false,
                            },
                            deleting: false,
                            dirty_count: Fetched::ready(0),
                            pull_request_error: None,
                            pull_request_loaded: true,
                            merged: false,
                            pull_request: Some(crate::domain::git_host::PrInfo {
                                number: self.prs.load(Ordering::SeqCst),
                                url: String::new(),
                            }),
                            tree: Fetched::default(),
                        }]),
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
                let count = if matches!(
                    target,
                    crate::usecase::state_subscription::SubscriptionTarget::Issues(_)
                ) {
                    &self.issues
                } else {
                    &self.prs
                };
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
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
                let usecase = StateSubscriptionUsecase::new(
                    vec![],
                    crate::test_support::state_subscription::read_driver(),
                )
                .with_reads(reads.clone(), None, vec![], String::new());
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
            let usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            )
            .with_reads(reads.clone(), None, vec![], String::new());
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
            assert!(!usecase.test_presenter().unwrap().test_runtime().inspect(
                |state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())
            ));
            assert_eq!(usecase.test_worker_count(), 0);
        }

        struct DisconnectingFiles {
            usecase: StateSubscriptionUsecase,
        }
        impl crate::domain::repository::file_watcher::FileWatchGateway for DisconnectingFiles {
            fn start_tree(
                &self,
                _: &str,
                _: crate::domain::repository::file_watcher::WatchChangeHandler,
            ) -> Result<u64, String> {
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
            let mut usecase = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
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
                String::new(),
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
            assert!(!usecase.test_presenter().unwrap().test_runtime().inspect(
                |state| state.registered(&SubscriptionTarget::RepositoryPaths.to_string())
            ));
            assert_eq!(usecase.test_worker_count(), 0);
        }
    }

    mod terminal_scenarios {
        use crate::usecase::state_subscription::*;
        use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
        use futures_util::StreamExt;
        use std::sync::Arc;

        use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor;
        use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
        use crate::domain::terminal_surface::entities::TerminalSurface;
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        use crate::infrastructure::state_subscription::Version;
        use crate::test_support::state_subscription::{
            terminal_item, Delivery, Event, StateSubscriptionEvent,
        };
        use crate::usecase::state_subscription::SubscriptionTarget;
        use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
        use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
        use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;
        use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;

        use crate::domain::terminal_surface::TerminalSurfaceOwner;
        use crate::domain::workspace_tree::WorkspaceIdentity;

        struct BlockingResetOutput {
            hub: Arc<TerminalSurfaceEventHub>,
            started: std::sync::mpsc::Sender<()>,
            release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        }

        impl TerminalSurfaceOutputControl for BlockingResetOutput {
            fn set_state_sink(
                &self,
                sink: Arc<dyn TerminalSurfaceStateSink>,
            ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
                self.hub.set_state_sink(sink)
            }

            fn initialize(
                &self,
                registration: crate::usecase::terminal_surface::output::TerminalRegistration,
            ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
                self.hub.initialize(registration)
            }

            fn subscribe_output(&self, session_key: &str, client: &str, units: usize) {
                if client == "stopping" && units == 0 {
                    self.started.send(()).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                self.hub.subscribe_output(session_key, client, units);
            }

            fn unsubscribe_output(&self, session_key: &str, client: &str) {
                self.hub.unsubscribe_output(session_key, client);
            }

            fn processed_output(&self, session_key: &str, client: &str, units: usize) {
                self.hub.processed_output(session_key, client, units);
            }
        }

        fn fixture() -> (
            crate::test_support::state_subscription::TerminalSubscriptions,
            Arc<TerminalSurfaceRuntimeGatewayFor>,
            Arc<TerminalSurfaceEventHub>,
            TerminalSurface,
        ) {
            let (terminal, gateway, hub, surface) =
                crate::test_support::state_subscription::terminal_application_fixture();
            let subscriptions = StateSubscriptionUsecase::new(
                vec!["/repo".into()],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal);
            (subscriptions, gateway, hub, surface)
        }

        #[tokio::test]
        async fn test_terminal購読_同じstreamでsnapshot差分と区切りを届ける() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let raw = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            let before = gateway.snapshot_materialization_count();
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&raw).unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            subscriptions
                .usecase
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        "repository-paths",
                    )
                    .unwrap(),
                    &format!("{}:{}", "client", "repository-paths"),
                    None,
                )
                .await
                .unwrap();
            // Then
            let mut terminal_snapshot = false;
            let mut paths_snapshot = false;
            let mut terminal_bookmark = false;
            let mut paths_bookmark = false;
            for _ in 0..4 {
                match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                    .await
                    .unwrap()
                {
                    Some(StateSubscriptionEvent::Item(
                        target,
                        Event::Snapshot(Version { sequence: 0, .. }, value),
                    )) if target == "input" => {
                        terminal_snapshot = matches!(
                            terminal_item(&value),
                            crate::adaptor::presenter::client::terminal_event::Item::Snapshot(_)
                        );
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Snapshot(_, _)))
                        if target == "client:repository-paths" =>
                    {
                        paths_snapshot = true
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_)))
                        if target == "input" =>
                    {
                        terminal_bookmark = true
                    }
                    Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_)))
                        if target == "client:repository-paths" =>
                    {
                        paths_bookmark = true
                    }
                    event => panic!("unexpected subscription event: {}", event.is_some()),
                }
            }
            assert!(terminal_snapshot && paths_snapshot && terminal_bookmark && paths_bookmark);
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
            for sequence in 1..=100 {
                surface
                    .record_output(surface.runtime_generation, std::time::Instant::now())
                    .unwrap();
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    1,
                    sequence,
                ))
                .unwrap();
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "🙂".into(),
                    sequence,
                });
            }
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
            for sequence in 1..=100 {
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(version, Delivery::Delta, value))) if version.sequence == sequence && matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
                );
            }
        }

        #[tokio::test]
        async fn test_terminal再開_履歴内ならsnapshotを作らず再起動後は作る() {
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) =
                stream.next().await
            else {
                panic!("snapshot");
            };
            stream.next().await;
            let before = gateway.snapshot_materialization_count();
            stop_terminal(
                &subscriptions,
                "client",
                &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            )
            .unwrap();
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "next".into(),
                sequence: 1,
            });
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input-2",
                    Some((&version.epoch, version.sequence)),
                )
                .await
                .unwrap();
            assert_eq!(gateway.snapshot_materialization_count(), before);
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(v, Delivery::Delta, _))) if v.sequence == 1)
            );
            stream.next().await;
            stop_terminal(
                &subscriptions,
                "client",
                &crate::usecase::state_subscription::SubscriptionTarget::parse(&target).unwrap(),
            )
            .unwrap();
            let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
            gateway.remove_surface(surface.runtime_generation.value());
            gateway.insert_surface(recreated.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &recreated.session_key,
                "/repo",
                None,
                2,
                0,
            ))
            .unwrap();
            subscriptions
                .test_presenter()
                .unwrap()
                .initialize(&crate::test_support::state_subscription::registration(
                    &recreated.session_key,
                    "/repo",
                    None,
                    recreated.runtime_generation.value(),
                    recreated.latest_sequence(),
                ))
                .unwrap();
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input-3",
                    Some((&version.epoch, version.sequence)),
                )
                .await
                .unwrap();
            assert!(
                matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(v, _))) if v.epoch != version.epoch)
            );
            assert_eq!(gateway.snapshot_materialization_count(), before + 1);
        }

        #[tokio::test]
        async fn test_terminal再開不能_配信開始直後の出力を現在snapshotで回復する() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
            stream.next().await;
            let cursor = Some(("old-epoch", 0));
            subscriptions
                .deps()
                .start_subscription("client", &target, "input", cursor)
                .await
                .unwrap();
            assert!(!hub.test_subscribed(&surface.session_key, "client"));
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                None
            );

            // When
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "next".into(),
                sequence: 1,
            });
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                None
            );
            // Then
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _)))
                    if version.sequence == 1
            ));
            assert!(hub.test_subscribed(&surface.session_key, "client"));
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                Some(0)
            );
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "x".repeat(100_001).into(),
                sequence: 2,
            });
            assert_eq!(
                hub.test_pending_amount(&surface.session_key, "client"),
                Some(100_001)
            );
        }

        #[tokio::test]
        async fn test_terminal購読_件数上限がなく停止と切断で流量を解放する() {
            let (subscriptions, gateway, hub, surface) = fixture();
            let stream = subscriptions.open("client".into()).unwrap();
            for index in 0..20 {
                let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(format!(
                    "/repo-{index}"
                )))
                .unwrap();
                let surface = TerminalSurface::new(index + 2, owner.clone(), None);
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    &format!("/repo-{index}"),
                    None,
                    surface.runtime_generation.value(),
                    surface.latest_sequence(),
                ))
                .unwrap();
                subscriptions
                    .deps()
                    .start_subscription(
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(
                            &SubscriptionTarget::Terminal(owner).to_string(),
                        )
                        .unwrap(),
                        &format!("input-{index}"),
                        None,
                    )
                    .await
                    .unwrap();
            }
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().len()),
                20
            );
            drop(stream);
            assert!(subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.active_targets().is_empty()));
            assert!(crate::test_support::state_subscription::terminal_processed(
                &subscriptions,
                "client",
                &SubscriptionTarget::Terminal(surface.owner).to_string(),
                5000
            )
            .is_err());
        }

        #[tokio::test]
        async fn test_terminal復元_同じ対象の全clientの流量停止を解放する() {
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            first.next().await;
            second.next().await;
            for client in ["first", "second"] {
                subscriptions
                    .deps()
                    .start_subscription(
                        client,
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                        client,
                        None,
                    )
                    .await
                    .unwrap();
            }
            first.next().await;
            first.next().await;
            second.next().await;
            second.next().await;
            for client in ["first", "second"] {
                hub.subscribe_output(
                    &surface.session_key,
                    client,
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
                );
            }
            subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .update(|state| state.require_delta_snapshot(&target))
                .unwrap();

            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
            let (sender, receiver) = std::sync::mpsc::channel();
            let hub_for_wait = hub.clone();
            let session_key = surface.session_key.clone();
            let waiter = std::thread::spawn(move || {
                hub_for_wait.wait_output(&session_key);
                sender.send(()).unwrap();
            });
            let resumed = receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .is_ok();
            hub.unsubscribe_output(&surface.session_key, "first");
            hub.unsubscribe_output(&surface.session_key, "second");
            waiter.join().unwrap();
            assert!(resumed);
        }

        #[tokio::test]
        async fn test_snapshot作成中_別terminalのsnapshotと出力とexecutorを止めない() {
            // Given
            use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
            let mut gateway = FakePtyGateway::new();
            let first = TerminalSurface::new(
                1,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/first")).unwrap(),
                None,
            );
            let second = TerminalSurface::new(
                2,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/second")).unwrap(),
                None,
            );
            gateway.additional_surfaces = vec![first.clone(), second.clone()];
            let gateway = Arc::new(gateway);
            let hub = Arc::new(TerminalSurfaceEventHub::new());
            hub.initialize(crate::test_support::state_subscription::registration(
                &first.session_key,
                "/first",
                None,
                1,
                0,
            ))
            .unwrap();
            hub.initialize(crate::test_support::state_subscription::registration(
                &second.session_key,
                "/second",
                None,
                2,
                0,
            ))
            .unwrap();
            let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub.clone(),
        ),
    );
            let subscriptions = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal.clone());
            let mut first_stream = Box::pin(subscriptions.open("first-client".into()).unwrap());
            let mut second_stream = Box::pin(subscriptions.open("second-client".into()).unwrap());
            first_stream.next().await;
            second_stream.next().await;
            let (started, waiting) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            *gateway.snapshot_gate.lock() = Some((started, blocked));
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "first-client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &SubscriptionTarget::Terminal(first.owner).to_string(),
                    )
                    .unwrap(),
                    "first-input",
                    None,
                )
                .await
                .unwrap();
            let request = tokio::spawn(async move { first_stream.next().await });
            tokio::task::spawn_blocking(move || {
                waiting
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap()
            })
            .await
            .unwrap();
            assert!(gateway.snapshot_gate.try_lock().is_some());
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                subscriptions.deps().start_subscription(
                    "second-client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &SubscriptionTarget::Terminal(second.owner.clone()).to_string(),
                    )
                    .unwrap(),
                    "second-input",
                    None,
                ),
            )
            .await;
            // Then
            assert!(!request.is_finished());
            assert!(result.unwrap().is_ok());
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.snapshot_requests("second-client")),
                vec![SubscriptionTarget::Terminal(second.owner.clone()).to_string()]
            );
            subscriptions.schedule_terminal_refresh(
                vec!["second-client".into()],
                SubscriptionTarget::Terminal(second.owner.clone()),
            );
            assert_eq!(subscriptions.test_worker_count(), 2);
            tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                .await
                .expect("second snapshot");
            tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                .await
                .expect("second bookmark");
            terminal.resize(&second.owner, 40, 120).unwrap();
            terminal
                .write_attached(&second.owner, "second-input", 0, "input")
                .unwrap();
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: second.session_key,
                data: "live".into(),
                sequence: 1,
            });
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
                    .await
                    .expect("second output"),
                Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(_, Delivery::Delta, _)
                ))
            ));
            release.send(()).unwrap();
            assert!(matches!(
                request.await.unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
        }

        #[tokio::test]
        async fn test_snapshot作成中_同じterminalへ追加されたclientにもsnapshotを届ける() {
            // Given
            use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
            let mut gateway = FakePtyGateway::new();
            let surface = TerminalSurface::new(
                1,
                TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
                None,
            );
            gateway.additional_surfaces = vec![surface.clone()];
            let gateway = Arc::new(gateway);
            let hub = Arc::new(TerminalSurfaceEventHub::new());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                0,
            ))
            .unwrap();
            let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub,
        ),
    );
            let subscriptions = StateSubscriptionUsecase::new(
                vec![],
                crate::test_support::state_subscription::read_driver(),
            );
            let subscriptions = subscriptions.with_terminal(terminal);
            let target = SubscriptionTarget::Terminal(surface.owner);
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            first.next().await;
            second.next().await;
            let (started, waiting) = std::sync::mpsc::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            *gateway.snapshot_gate.lock() = Some((started, blocked));

            // When
            subscriptions
                .deps()
                .start_subscription("first", &target, "first-input", None)
                .await
                .unwrap();
            let first_event = tokio::spawn(async move { first.next().await });
            tokio::task::spawn_blocking(move || {
                waiting
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .unwrap()
            })
            .await
            .unwrap();
            subscriptions
                .deps()
                .start_subscription("second", &target, "second-input", None)
                .await
                .unwrap();
            subscriptions.schedule_terminal_refresh(vec!["second".into()], target.clone());
            assert_eq!(subscriptions.test_worker_count(), 1);
            release.send(()).unwrap();

            // Then
            assert!(matches!(
                first_event.await.unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
            assert!(matches!(
                tokio::time::timeout(std::time::Duration::from_secs(1), second.next())
                    .await
                    .unwrap(),
                Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
            ));
        }

        #[tokio::test]
        async fn test_terminal復元_停止と切断の競合でも停止済みclientを再登録しない() {
            use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;

            for close in [false, true] {
                // Given
                let surface = TerminalSurface::new(
                    1,
                    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
                    None,
                );
                let mut gateway = FakePtyGateway::new();
                gateway.additional_surfaces.push(surface.clone());
                let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    1,
                    0,
                ))
                .unwrap();
                let (started, waiting) = std::sync::mpsc::channel();
                let (release, blocked) = std::sync::mpsc::channel();
                let output = Arc::new(BlockingResetOutput {
                    hub: hub.clone(),
                    started,
                    release: std::sync::Mutex::new(blocked),
                });
                let terminal = Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            Arc::new(gateway),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            output,
        ));
                let subscriptions = StateSubscriptionUsecase::new(
                    vec![],
                    crate::test_support::state_subscription::read_driver(),
                );
                let subscriptions = subscriptions.with_terminal(terminal);
                let target = SubscriptionTarget::Terminal(surface.owner.clone());
                let _stopping = subscriptions.open("stopping".into()).unwrap();
                let _active = subscriptions.open("active".into()).unwrap();
                subscriptions
                    .deps()
                    .start_subscription("stopping", &target, "stopping-input", None)
                    .await
                    .unwrap();
                subscriptions
                    .deps()
                    .start_subscription("active", &target, "active-input", None)
                    .await
                    .unwrap();
                hub.subscribe_output(
                    &surface.session_key,
                    "active",
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
                );

                // When
                subscriptions.schedule_terminal_refresh(
                    vec!["stopping".into(), "active".into()],
                    target.clone(),
                );
                tokio::task::spawn_blocking(move || {
                    waiting
                        .recv_timeout(std::time::Duration::from_secs(2))
                        .unwrap()
                })
                .await
                .unwrap();
                let stopping = subscriptions.clone();
                let stopped_target = target.clone();
                let (stop_started, started_stop) = tokio::sync::oneshot::channel();
                let mut stop = tokio::task::spawn_blocking(move || {
                    let _ = stop_started.send(());
                    if close {
                        stopping.close_client("stopping");
                    } else {
                        stop_terminal(&stopping, "stopping", &stopped_target).unwrap();
                    }
                });
                started_stop.await.unwrap();
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(100), &mut stop)
                        .await
                        .is_err()
                );
                release.send(()).unwrap();
                stop.await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    while subscriptions.test_worker_count() != 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();

                // Then
                assert!(!hub.test_subscribed(&surface.session_key, "stopping"));
                assert!(hub.test_subscribed(&surface.session_key, "active"));
                let units =
                    crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1;
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "x".repeat(units).into(),
                    sequence: 1,
                });
                hub.processed_output(&surface.session_key, "active", units);
                let (resumed, wait) = std::sync::mpsc::channel();
                let session_key = surface.session_key.clone();
                let waiting_hub = hub.clone();
                let waiter = std::thread::spawn(move || {
                    waiting_hub.wait_output(&session_key);
                    resumed.send(()).unwrap();
                });
                let active_resumed = wait.recv_timeout(std::time::Duration::from_secs(2)).is_ok();
                hub.unsubscribe_output(&surface.session_key, "active");
                hub.unsubscribe_output(&surface.session_key, "stopping");
                waiter.join().unwrap();
                assert!(active_resumed);
            }
        }

        #[tokio::test]
        async fn test_terminal差分_出力の重複を除き同じ出力番号で寸法と終了を配信する() {
            // Given
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            // When
            let output = TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                data: "x".into(),
                sequence: 1,
            };
            hub.publish(output.clone());
            hub.publish(output);
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                cols: 120,
                rows: 30,
                sequence: 1,
            });
            hub.publish(TerminalSurfaceOutputEvent::Exit {
                session_key: surface.session_key.clone(),
                runtime_generation: 1,
                exit_code: Some(7),
                sequence: 1,
            });
            let mut actual = Vec::new();
            for _ in 0..3 {
                actual.push(stream.next().await.unwrap());
            }
            // Then
            for ((sequence, expected), event) in [
                (
                    1,
                    TerminalSurfaceStreamItem::Output {
                        session_key: surface.session_key.clone(),
                        data: "x".into(),
                        sequence: 1,
                    },
                ),
                (
                    1,
                    TerminalSurfaceStreamItem::Resize {
                        session_key: surface.session_key.clone(),
                        cols: 120,
                        rows: 30,
                        sequence: 1,
                    },
                ),
                (
                    1,
                    TerminalSurfaceStreamItem::Exit {
                        session_key: surface.session_key.clone(),
                        exit_code: Some(7),
                        sequence: 1,
                    },
                ),
            ]
            .into_iter()
            .zip(actual)
            {
                let StateSubscriptionEvent::Item(
                    actual_target,
                    Event::Change(version, Delivery::Delta, value),
                ) = event
                else {
                    panic!("terminal delta");
                };
                let expected_payload = crate::adaptor::presenter::state_subscription_wire::payload(
                    &StateValue::Terminal(expected.into()),
                )
                .unwrap();
                assert_eq!(actual_target, "input");
                assert_eq!(version.sequence, sequence);
                assert_eq!(*value, expected_payload.into());
            }
        }

        #[tokio::test]
        async fn test_terminal購読_出力前の寸法変更と終了を版ゼロで届け再開する() {
            // Given
            let (subscriptions, _, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) =
                stream.next().await
            else {
                panic!("snapshot");
            };
            assert_eq!(version.sequence, 0);
            stream.next().await;
            // When
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                cols: 120,
                rows: 30,
                sequence: 0,
            });
            hub.publish(TerminalSurfaceOutputEvent::Exit {
                session_key: surface.session_key.clone(),
                runtime_generation: 1,
                exit_code: Some(7),
                sequence: 0,
            });
            // Then
            for reconnect in [false, true] {
                if reconnect {
                    stop_terminal(
                        &subscriptions,
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                    )
                    .unwrap();
                    subscriptions
                        .deps()
                        .start_subscription(
                            "client",
                            &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                                .unwrap(),
                            "input",
                            Some((&version.epoch, version.sequence)),
                        )
                        .await
                        .unwrap();
                }
                let Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(resize_version, Delivery::Delta, resize),
                )) = stream.next().await
                else {
                    panic!("resize");
                };
                assert_eq!(resize_version, version);
                assert!(matches!(
                    terminal_item(&resize),
                    crate::adaptor::presenter::client::terminal_event::Item::Resize(
                        crate::adaptor::presenter::client::TerminalResize {
                            cols: 120,
                            rows: 30,
                            sequence: 0,
                            ..
                        }
                    )
                ));
                let Some(StateSubscriptionEvent::Item(
                    _,
                    Event::Change(exit_version, Delivery::Delta, exit),
                )) = stream.next().await
                else {
                    panic!("exit");
                };
                assert_eq!(exit_version, version);
                assert!(matches!(
                    terminal_item(&exit),
                    crate::adaptor::presenter::client::terminal_event::Item::Exit(
                        crate::adaptor::presenter::client::TerminalExit {
                            exit_code: Some(7),
                            sequence: 0,
                            ..
                        }
                    )
                ));
            }
        }

        #[tokio::test]
        async fn test_terminal購読_出力と寸法の逆転は古い寸法を捨てずsnapshotで復元する() {
            // Given
            let (subscriptions, gateway, hub, mut surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await;
            stream.next().await;
            surface
                .record_output(surface.runtime_generation, std::time::Instant::now())
                .unwrap();
            surface.checkpoint.cols = 120;
            surface.checkpoint.rows = 30;
            gateway.insert_surface(surface.clone());
            hub.initialize(crate::test_support::state_subscription::registration(
                &surface.session_key,
                "/repo",
                None,
                1,
                1,
            ))
            .unwrap();
            // When
            hub.publish(TerminalSurfaceOutputEvent::Output {
                session_key: surface.session_key.clone(),
                sequence: 1,
                data: "x".into(),
            });
            hub.publish(TerminalSurfaceOutputEvent::Resize {
                session_key: surface.session_key.clone(),
                sequence: 0,
                cols: 120,
                rows: 30,
            });
            // Then
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, snapshot))) =
                stream.next().await
            else {
                panic!("resynchronized snapshot");
            };
            assert_eq!(version.sequence, 1);
            assert!(
                matches!(terminal_item(&snapshot), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(value) if value.cols == 120 && value.rows == 30)
            );
        }

        #[tokio::test]
        async fn test_terminal削除_経路と履歴を解放し購読と入力は明示停止まで保つ() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            // When
            for generation in 1..=10 {
                let surface = TerminalSurface::new(generation, surface.owner.clone(), None);
                gateway.insert_surface(surface.clone());
                hub.initialize(crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    generation,
                    0,
                ))
                .unwrap();
                subscriptions
                    .deps()
                    .start_subscription(
                        "client",
                        &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                            .unwrap(),
                        "input",
                        None,
                    )
                    .await
                    .unwrap();
                stream.next().await;
                stream.next().await;
                hub.publish(TerminalSurfaceOutputEvent::Output {
                    session_key: surface.session_key.clone(),
                    data: "retained".into(),
                    sequence: 1,
                });
                hub.publish(TerminalSurfaceOutputEvent::Exit {
                    session_key: surface.session_key.clone(),
                    runtime_generation: generation,
                    exit_code: Some(0),
                    sequence: 1,
                });
                gateway.remove_surface(generation).unwrap();
                // Then
                assert!(
                    subscriptions
                        .test_presenter()
                        .unwrap()
                        .test_runtime()
                        .inspect(|state| usize::from(state.registered(&target)))
                        == 0
                );
                let runtime = &subscriptions.test_presenter().unwrap().test_runtime();
                assert_eq!(runtime.inspect(|state| state.active_targets().len()), 1);
                assert!(runtime.inspect(|state| state.current_version(&target).is_none()));
                runtime.mutate(|state| (state.bookmark("client"), true));
                assert!(runtime.inspect(|state| state.snapshot_requests("client").is_empty()));
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
                );
                assert!(
                    matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Exit(event) if event.exit_code == Some(0)))
                );
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state
                        .lookup("input")
                        .is_some_and(|(client, raw)| client == "client" && raw == target)));
                assert_eq!(
                    subscriptions
                        .test_presenter()
                        .as_ref()
                        .unwrap()
                        .test_runtime()
                        .inspect(|state| state.active_targets().len()),
                    1
                );
                stop_terminal(
                    &subscriptions,
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                )
                .unwrap();
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().is_empty()));
                assert!(subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.active_targets().is_empty()));
            }
        }

        #[tokio::test]
        async fn test_terminal購読開始_古いsummary取得後の再作成でepochを巻き戻さない() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            stream.next().await;
            let old_version = subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.current_version(&target))
                .unwrap();
            let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
            let replacement = recreated.clone();
            let replacement_key = recreated.session_key.clone();
            let gateway_for_replacement = gateway.clone();
            let hub_for_replacement = hub.clone();
            *gateway.before_output_order.lock() = Some(Box::new(move || {
                gateway_for_replacement.remove_surface(1).unwrap();
                gateway_for_replacement.insert_surface(replacement);
                hub_for_replacement
                    .initialize(crate::test_support::state_subscription::registration(
                        &replacement_key,
                        "/repo",
                        None,
                        2,
                        0,
                    ))
                    .unwrap();
            }));
            // When
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(&target)
                        .unwrap(),
                    "new-input",
                    Some((&old_version.epoch, old_version.sequence)),
                )
                .await
                .unwrap();
            // Then
            let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, value))) =
                stream.next().await
            else {
                panic!("new runtime snapshot");
            };
            assert_ne!(version.epoch, old_version.epoch);
            assert_eq!(
                version,
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.current_version(&target))
                    .unwrap()
            );
            assert!(
                matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(surface) if surface.session_key == recreated.session_key)
            );
            subscriptions.test_presenter().unwrap().remove(
                &crate::test_support::state_subscription::registration(
                    &surface.session_key,
                    "/repo",
                    None,
                    surface.runtime_generation.value(),
                    0,
                ),
            );
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| state.current_version(&target)),
                Some(version)
            );
            assert_eq!(
                subscriptions
                    .test_presenter()
                    .unwrap()
                    .test_runtime()
                    .inspect(|state| usize::from(state.registered(&target))),
                1
            );
        }

        #[tokio::test]
        async fn test_terminal購読開始_出力順序区間内でsummary取得に失敗したら開始しない() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let session_key = surface.session_key.clone();
            let _stream = subscriptions.open("client".into()).unwrap();
            let removed = gateway.clone();
            *gateway.during_output_order.lock() = Some(Box::new(move || {
                removed
                    .remove_surface(surface.runtime_generation.value())
                    .unwrap();
            }));

            // When
            let result = subscriptions
                .deps()
                .start_subscription("client", &target, "input", None)
                .await;

            // Then
            assert!(result.is_err());
            assert!(subscriptions
                .terminal
                .test_input_id("client", &target)
                .is_none());
            assert!(matches!(
                subscriptions
                    .terminal
                    .terminal_processed("input", 5000)
                    .unwrap_err()
                    .source,
                crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
            ));
            assert!(!hub.test_subscribed(&session_key, "client"));
            assert!(!subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state
                    .lookup("input")
                    .is_some_and(|(client, raw)| client == "client" && raw == target.to_string())));
        }

        #[tokio::test]
        async fn test_terminal購読開始_途中でclientが切断したら出力登録を巻き戻す() {
            // Given
            let (subscriptions, gateway, hub, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let _stream = subscriptions.open("client".into()).unwrap();
            let disconnected = subscriptions.clone();
            *gateway.before_output_order.lock() =
                Some(Box::new(move || disconnected.close_client("client")));
            // When
            let result = subscriptions
                .deps()
                .start_subscription("client", &target, "input", None)
                .await;
            // Then
            assert!(
                matches!(result, Err(crate::usecase::state_subscription::StateReadError {
        source: crate::usecase::state_subscription::StateReadFailure::Subscription(error), ..
    }) if *error == crate::usecase::state_subscription::SubscriptionError::StreamEnded)
            );
            assert!(!hub.test_subscribed(&surface.session_key, "client"));
            assert!(subscriptions
                .terminal
                .test_input_id("client", &target)
                .is_none());
            assert!(matches!(
                subscriptions
                    .terminal
                    .terminal_processed("input", 5000)
                    .unwrap_err()
                    .source,
                crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
            ));
            assert!(!subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .inspect(|state| state
                    .lookup("input")
                    .is_some_and(|(client, raw)| client == "client" && raw == target.to_string())));
        }

        #[tokio::test]
        async fn test_terminal再取得失敗_開始済み購読へ失敗を届ける() {
            // Given
            use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
            let (subscriptions, _, hub, surface) = fixture();
            let mut gateway = FakePtyGateway::new();
            gateway.surface = Some(surface.clone());
            let gateway = Arc::new(gateway);
            let terminal = Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
        Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        gateway.clone(),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
    ));
            let subscriptions = subscriptions.with_terminal(terminal);
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let stream = subscriptions.open("client".into()).unwrap();
            tokio::pin!(stream);
            assert!(matches!(
                stream.next().await,
                Some(StateSubscriptionEvent::Ready)
            ));
            subscriptions
                .deps()
                .start_subscription(
                    "client",
                    &crate::usecase::state_subscription::SubscriptionTarget::parse(
                        &target.to_string(),
                    )
                    .unwrap(),
                    "input",
                    None,
                )
                .await
                .unwrap();
            stream.next().await.unwrap();
            stream.next().await.unwrap();
            *gateway.snapshot_unavailable.lock() = true;
            // When
            subscriptions.schedule_terminal_refresh(vec!["client".into()], target.clone());
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
            // Then
            assert!(
                matches!(event, StateSubscriptionEvent::Item(_, Event::Change(_, _, value)) if matches!(value.as_ref(), crate::adaptor::presenter::state_subscription::PublishedState::Failure(failure) if failure.message.contains("snapshot unavailable")))
            );
        }

        #[tokio::test]
        async fn test_terminal購読開始_捨てられた最初の状態を再要求から作り直して届ける() {
            // Given
            let (subscriptions, _, _, surface) = fixture();
            let target = SubscriptionTarget::Terminal(surface.owner.clone());
            let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
            first.next().await.unwrap();
            subscriptions
                .deps()
                .start_subscription("first", &target, "first-input", None)
                .await
                .unwrap();
            let snapshot = tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                snapshot,
                StateSubscriptionEvent::Item(_, Event::Snapshot(_, _))
            ));
            subscriptions
                .terminal
                .stop_delivery(
                    "first",
                    &target,
                    "first-input",
                    &subscriptions
                        .usecase
                        .test_presenter()
                        .unwrap()
                        .delivery("first-input")
                        .unwrap()
                        .2,
                )
                .unwrap();
            let runtime = subscriptions.test_presenter().unwrap().test_runtime();
            let raw = target.to_string();
            assert!(runtime.inspect(|state| state.current_version(&raw).is_some()));
            assert!(runtime.inspect(|state| state.needs_snapshot(&raw, None).unwrap()));
            let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
            second.next().await.unwrap();
            // When
            subscriptions
                .deps()
                .start_subscription("second", &target, "second-input", None)
                .await
                .unwrap();
            assert_eq!(
                runtime.inspect(|state| state.snapshot_requests("second")),
                vec![raw.clone()]
            );
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), second.next())
                .await
                .unwrap()
                .unwrap();
            // Then
            assert!(
                matches!(event, StateSubscriptionEvent::Item(delivered, Event::Snapshot(_, value))
        if delivered == "second-input" && matches!(terminal_item(&value),
            crate::adaptor::presenter::client::terminal_event::Item::Snapshot(snapshot) if snapshot.session_key == surface.session_key))
            );
        }

        fn stop_terminal(
            subscriptions: &crate::test_support::state_subscription::TerminalSubscriptions,
            client: &str,
            target: &SubscriptionTarget,
        ) -> Result<(), crate::usecase::state_subscription::SubscriptionError> {
            let Some(id) = subscriptions.terminal.test_input_id(client, target) else {
                return Ok(());
            };
            let Some((_, _, delivery)) = subscriptions
                .usecase
                .test_presenter()
                .unwrap()
                .delivery(&id)
            else {
                return Ok(());
            };
            subscriptions
                .terminal
                .stop_delivery(client, target, &id, &delivery)
        }
    }
}

mod terminal_composition {
    use crate::usecase::state_subscription::*;
    use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
    use crate::usecase::terminal_surface::application::TerminalSurfaceStreamItem;
    use crate::usecase::terminal_surface::subscription::*;
    use crate::usecase::terminal_surface::test_helpers::subscription::*;
    use parking_lot::Mutex;
    use std::collections::HashSet;
    use std::sync::Arc;
    #[tokio::test]
    async fn test_terminal入力識別子_明示した購読識別子で開始する() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, _, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        // When
        usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await
            .unwrap();
        // Then
        assert_eq!(
            usecase.test_input_id("client", &target).as_deref(),
            Some("input")
        );
        gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .unwrap();
    }

    #[tokio::test]
    async fn test_terminal開始_配信登録前の停止を検出し全登録を戻す() {
        // Given
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(12);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        let stopped = usecase.clone();
        let stopping_target = target.clone();
        let stopping_output = output.clone();
        *gateway.before_output_order.lock() = Some(Box::new(move || {
            stopped
                .stop_delivery(
                    "client",
                    &stopping_target,
                    "input",
                    &FakeDelivery {
                        output: &stopping_output,
                        client: "client",
                        target: &stopping_target,
                        input: "input",
                    },
                )
                .unwrap()
        }));
        // When
        let result = usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await;
        // Then
        assert!(
            matches!(result.unwrap_err().source, StateReadFailure::Subscription(error) if *error == SubscriptionError::StreamEnded)
        );
        assert_ended(&usecase, "client", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        assert!(gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .is_err());
        assert_eq!(*output.starts.lock(), 1);
        assert_eq!(*output.stops.lock(), 2);
    }

    #[tokio::test]
    async fn test_terminal開始_出力順序区間後の停止を検出し二重解除できる() {
        // Given
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(12);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        let stopped = usecase.clone();
        let stopping_target = target.clone();
        let stopping_hub = hub.clone();
        let session = surface.session_key.clone();
        let stopping_output = output.clone();
        *gateway.after_output_order.lock() = Some(Box::new(move || {
            assert!(stopping_hub.test_subscribed(&session, "client"));
            assert_eq!(stopping_output.subscribed.lock().len(), 1);
            stopped
                .stop_delivery(
                    "client",
                    &stopping_target,
                    "input",
                    &FakeDelivery {
                        output: &stopping_output,
                        client: "client",
                        target: &stopping_target,
                        input: "input",
                    },
                )
                .unwrap();
        }));
        // When
        let result = usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await;
        // Then
        assert!(
            matches!(result.unwrap_err().source, StateReadFailure::Subscription(error) if *error == SubscriptionError::StreamEnded)
        );
        assert_ended(&usecase, "client", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        assert!(gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .is_err());
        assert_eq!(*output.stops.lock(), 2);
    }

    #[tokio::test]
    async fn test_terminal開始_世代の再作成時は新しい出力順序で開始し直す() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, _, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner.clone());
        usecase.open_client("client".into()).unwrap();
        let recreated =
            crate::domain::terminal_surface::entities::TerminalSurface::new(2, surface.owner, None);
        let replacement = gateway.clone();
        *gateway.before_output_order.lock() = Some(Box::new(move || {
            replacement.remove_surface(1).unwrap();
            replacement.insert_surface(recreated);
        }));
        // When
        usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await
            .unwrap();
        // Then
        assert_eq!(*output.starts.lock(), 1);
        assert_eq!(
            usecase.test_input_id("client", &target).as_deref(),
            Some("input")
        );
        gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .unwrap();
    }

    #[tokio::test]
    async fn test_terminal開始_順序区間内の世代変化でも新しい世代で開始する() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, _, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner.clone());
        usecase.open_client("client".into()).unwrap();
        let recreated =
            crate::domain::terminal_surface::entities::TerminalSurface::new(2, surface.owner, None);
        let replacement = gateway.clone();
        *gateway.during_output_order.lock() = Some(Box::new(move || {
            replacement.remove_surface(1).unwrap();
            replacement.insert_surface(recreated);
        }));
        // When
        usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await
            .unwrap();
        // Then
        assert_eq!(*output.starts.lock(), 1);
        assert_eq!(
            usecase.test_input_id("client", &target).as_deref(),
            Some("input")
        );
    }

    #[tokio::test]
    async fn test_terminal開始失敗_summary取得失敗で記録と配信を戻す() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        let removed = gateway.clone();
        *gateway.during_output_order.lock() = Some(Box::new(move || {
            removed.remove_surface(1).unwrap();
        }));
        // When
        assert!(usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input"
                }
            )
            .await
            .is_err());
        // Then
        assert_ended(&usecase, "client", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
        assert_eq!(*output.starts.lock(), 0);
        assert_eq!(*output.stops.lock(), 1);
        assert!(gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .is_err());
    }

    #[tokio::test]
    async fn test_terminal開始失敗_配信の失敗で記録と出力購読を戻す() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.start_error.lock() = Some(SubscriptionError::UnknownTarget);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        // When
        let error = usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await
            .unwrap_err();
        // Then
        assert!(
            matches!(error.source, StateReadFailure::Subscription(error) if *error == SubscriptionError::UnknownTarget)
        );
        assert_ended(&usecase, "client", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
        assert_eq!(*output.stops.lock(), 1);
        assert!(gateway
            .write_attached(&surface.session_key, "input", 1, "input")
            .is_err());
    }

    #[tokio::test]
    async fn test_terminal処理報告_購読中の流量制御へ渡し停止後は拒む() {
        // Given
        let (terminal, _, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(6000);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        usecase
            .start_subscription(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .await
            .unwrap();
        // When
        usecase.terminal_processed("input", 5000).unwrap();
        // Then
        assert_eq!(
            hub.test_pending_amount(&surface.session_key, "client"),
            Some(1000)
        );
        usecase
            .stop_delivery(
                "client",
                &target,
                "input",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "input",
                },
            )
            .unwrap();
        assert_ended(&usecase, "client", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
    }

    #[tokio::test]
    async fn test_terminal処理報告_同じclientの購読が複数でも量は最後の購読の報告だけで引く() {
        // Given
        let (terminal, _, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(6000);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );
        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        for input in ["first", "second"] {
            usecase
                .start_subscription(
                    "client",
                    &target,
                    input,
                    &FakeDelivery {
                        output: &output,
                        client: "client",
                        target: &target,
                        input,
                    },
                )
                .await
                .unwrap();
        }
        // When
        usecase.terminal_processed("first", 5000).unwrap();
        usecase.terminal_processed("second", 5000).unwrap();
        // Then
        assert_eq!(
            hub.test_pending_amount(&surface.session_key, "client"),
            Some(1000)
        );
    }

    #[tokio::test]
    async fn test_terminal作り直し予約_一つのworkerで追加clientのresetも併合する() {
        // Given
        use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
        let (_, _, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let mut gateway = FakePtyGateway::new();
        gateway.surface = Some(surface.clone());
        let gateway = Arc::new(gateway);
        let terminal = crate::test_support::state_subscription::terminal_application_with_gateway(
            gateway.clone(),
            hub.clone(),
        );
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(6000);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        let target = SubscriptionTarget::Terminal(surface.owner);
        for client in ["first", "second"] {
            usecase.open_client(client.into()).unwrap();
            usecase
                .start_subscription(
                    client,
                    &target,
                    client,
                    &FakeDelivery {
                        output: &output,
                        client: client,
                        target: &target,
                        input: client,
                    },
                )
                .await
                .unwrap();
        }
        let (started, entered) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        *gateway.snapshot_gate.lock() = Some((started, blocked));
        // When
        usecase.schedule_terminal_refresh(vec!["first".into()], target.clone());
        tokio::task::spawn_blocking(move || {
            entered
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        })
        .await
        .unwrap();
        usecase.schedule_terminal_refresh(vec!["second".into()], target.clone());
        // Then
        assert_eq!(usecase.test_worker_count(), 1);
        release.send(()).unwrap();
        wait_workers(&usecase).await;
        assert!(reset_clients(&usecase).lock().is_empty());
        assert_eq!(
            hub.test_pending_amount(&surface.session_key, "first"),
            Some(0)
        );
        assert_eq!(
            hub.test_pending_amount(&surface.session_key, "second"),
            Some(0)
        );
        let snapshots = output.snapshots.lock();
        assert_eq!(snapshots.len(), 1);
        assert!(
            matches!(&snapshots[0], (1, 0, StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(snapshot))) if snapshot.session_key == surface.session_key)
        );
    }

    #[tokio::test]
    async fn test_terminal作り直し失敗_失敗を配信してworkerを消す() {
        // Given
        use crate::usecase::terminal_surface::test_helpers::FakePtyGateway;
        let (_, _, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let mut gateway = FakePtyGateway::new();
        gateway.surface = Some(surface.clone());
        *gateway.snapshot_unavailable.lock() = true;
        let terminal = crate::test_support::state_subscription::terminal_application_with_gateway(
            Arc::new(gateway),
            hub,
        );
        let output = Arc::new(FakeOutput::default());
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );

        // When
        usecase.schedule_terminal_refresh(
            vec!["client".into()],
            SubscriptionTarget::Terminal(surface.owner),
        );
        wait_workers(&usecase).await;
        // Then
        assert!(output.snapshots.lock().is_empty());
        let failures = output.failures.lock();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].message.contains("snapshot unavailable"));
    }

    #[tokio::test]
    async fn test_terminal切断_閉じたclientをreset記録から消し残る購読者を保持する() {
        // Given
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(6000);
        let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal), requests);

        let target = SubscriptionTarget::Terminal(surface.owner);
        for client in ["closed", "active"] {
            usecase.open_client(client.into()).unwrap();
            usecase
                .start_subscription(
                    client,
                    &target,
                    client,
                    &FakeDelivery {
                        output: &output,
                        client: client,
                        target: &target,
                        input: client,
                    },
                )
                .await
                .unwrap();
        }
        reset_clients(&usecase).lock().insert(
            target.clone(),
            HashSet::from(["closed".into(), "active".into()]),
        );
        usecase.schedule_terminal_refresh(vec![], target.clone());
        let mut request = receiver.recv().await.unwrap();
        // When
        usecase.close_client("closed");
        // Then
        assert_ended(&usecase, "closed", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "closed"));
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        assert!(gateway
            .write_attached(&surface.session_key, "closed", 1, "input")
            .is_err());
        assert_eq!(
            reset_clients(&usecase).lock()[&target],
            HashSet::from(["active".into()])
        );
        assert_eq!(usecase.test_worker_count(), 1);
        assert!(matches!(
            request.cancelled.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        // When
        usecase.close_client("active");
        // Then
        assert_ended(&usecase, "active", &target);
        assert!(!hub.test_subscribed(&surface.session_key, "active"));
        assert!(reset_clients(&usecase).lock().is_empty());
        assert_eq!(usecase.test_worker_count(), 0);
        assert!(matches!(
            request.cancelled.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }

    #[tokio::test]
    async fn test_terminal購読共有_片方の停止で出力と新しい入力の宛先を外さない() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(12);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );
        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        usecase
            .start_subscription(
                "client",
                &target,
                "x",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "x",
                },
            )
            .await
            .unwrap();
        usecase
            .start_subscription(
                "client",
                &target,
                "y",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "y",
                },
            )
            .await
            .unwrap();
        assert!(gateway
            .write_attached(&surface.session_key, "x", 0, "old")
            .is_err());
        // When
        usecase
            .stop_delivery(
                "client",
                &target,
                "x",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "x",
                },
            )
            .unwrap();
        // Then
        assert!(hub.test_subscribed(&surface.session_key, "client"));
        assert_eq!(output.subscribed.lock().len(), 1);
        gateway
            .write_attached(&surface.session_key, "y", 1, "new")
            .unwrap();
        usecase.terminal_processed("y", 5000).unwrap();
        usecase
            .stop_delivery(
                "client",
                &target,
                "y",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "y",
                },
            )
            .unwrap();
        assert!(!hub.test_subscribed(&surface.session_key, "client"));
        assert!(output.subscribed.lock().is_empty());
        assert!(gateway
            .write_attached(&surface.session_key, "y", 2, "stopped")
            .is_err());
    }

    #[tokio::test]
    async fn test_terminal購読再利用_同じclientとidの新しい入力先へ古い停止が作用しない() {
        // Given
        use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
        use crate::usecase::state_subscription::StateSubscriptionDelivery;
        let (terminal, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let subscriptions =
            crate::test_support::state_subscription::test_subscriptions().with_terminal(terminal);
        let deps = subscriptions.deps();
        let _stream = deps.stream("client".into()).unwrap();
        let target = SubscriptionTarget::Terminal(surface.owner.clone());
        subscriptions
            .presenter
            .set_snapshot(
                &target,
                1,
                0,
                StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface.clone().into())),
            )
            .unwrap();
        deps.start_subscription("client", &target, "x", None)
            .await
            .unwrap();
        let (_, _, old) = subscriptions
            .usecase
            .test_presenter()
            .unwrap()
            .delivery("x")
            .unwrap();
        deps.stop_subscription("x").await.unwrap();
        subscriptions
            .presenter
            .set_snapshot(
                &target,
                1,
                0,
                StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface.clone().into())),
            )
            .unwrap();
        deps.start_subscription("client", &target, "x", None)
            .await
            .unwrap();
        // When
        subscriptions
            .terminal
            .stop_delivery("client", &target, "x", &old)
            .unwrap();
        old.finish(&Default::default()).unwrap();
        // Then
        assert!(subscriptions
            .usecase
            .test_presenter()
            .unwrap()
            .delivery("x")
            .is_some());
        assert!(hub.test_subscribed(&surface.session_key, "client"));
        gateway
            .write_attached(&surface.session_key, "x", 1, "new")
            .unwrap();
    }

    #[tokio::test]
    async fn test_terminal再開始_旧停止の後始末が新購読の流量制御を削除しない() {
        use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
        use crate::usecase::terminal_surface::error::UsecaseError;
        use crate::usecase::terminal_surface::output::{
            TerminalRegistration, TerminalSurfaceOutputControl, TerminalSurfaceStateSink,
        };
        struct BlockingOutput {
            hub: Arc<TerminalSurfaceEventHub>,
            entered: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl TerminalSurfaceOutputControl for BlockingOutput {
            fn set_state_sink(
                &self,
                sink: Arc<dyn TerminalSurfaceStateSink>,
            ) -> Result<(), UsecaseError> {
                self.hub.set_state_sink(sink)
            }
            fn initialize(&self, registration: TerminalRegistration) -> Result<(), UsecaseError> {
                self.hub.initialize(registration)
            }
            fn subscribe_output(&self, session: &str, client: &str, units: usize) {
                self.hub.subscribe_output(session, client, units);
            }
            fn unsubscribe_output(&self, session: &str, client: &str) {
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                self.hub.unsubscribe_output(session, client);
            }
            fn processed_output(&self, session: &str, client: &str, units: usize) {
                self.hub.processed_output(session, client, units);
            }
        }
        // Given
        let (_, gateway, hub, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let (entered, stopping) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let terminal = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(crate::usecase::test_helpers::NoopPerformance), gateway,
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        Arc::new(BlockingOutput { hub: hub.clone(), entered, release: Mutex::new(released) }),
    ));
        let output = Arc::new(FakeOutput::default());
        *output.pending.lock() = Some(6000);
        let usecase = TerminalSubscriptionUsecase::new(
            output.clone(),
            Some(terminal),
            crate::test_support::state_subscription::terminal_driver(),
        );
        let target = SubscriptionTarget::Terminal(surface.owner);
        usecase.open_client("client".into()).unwrap();
        usecase
            .start_subscription(
                "client",
                &target,
                "x",
                &FakeDelivery {
                    output: &output,
                    client: "client",
                    target: &target,
                    input: "x",
                },
            )
            .await
            .unwrap();
        // When
        let stop = std::thread::spawn({
            let usecase = usecase.clone();
            let target = target.clone();
            let output = output.clone();
            move || {
                usecase
                    .stop_delivery(
                        "client",
                        &target,
                        "x",
                        &FakeDelivery {
                            output: &output,
                            client: "client",
                            target: &target,
                            input: "x",
                        },
                    )
                    .unwrap()
            }
        });
        stopping
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let cleanup_holds_clients = clients_locked(&usecase);
        let (attempted, starting) = std::sync::mpsc::channel();
        let start = std::thread::spawn({
            let usecase = usecase.clone();
            let target = target.clone();
            let output = output.clone();
            let runtime = tokio::runtime::Handle::current();
            move || {
                attempted.send(()).unwrap();
                runtime
                    .block_on(usecase.start_subscription(
                        "client",
                        &target,
                        "y",
                        &FakeDelivery {
                            output: &output,
                            client: "client",
                            target: &target,
                            input: "y",
                        },
                    ))
                    .unwrap();
            }
        });
        starting
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        release.send(()).unwrap();
        stop.join().unwrap();
        start.join().unwrap();
        usecase.terminal_processed("y", 5000).unwrap();
        // Then
        assert!(cleanup_holds_clients);
        assert!(hub.test_subscribed(&surface.session_key, "client"));
        assert_eq!(
            hub.test_pending_amount(&surface.session_key, "client"),
            Some(1000)
        );
    }

    #[tokio::test]
    async fn test_terminal処理報告_購読識別子だけで受理し未知と汎用と停止済みを拒む() {
        // Given
        let (terminal, _, _, surface) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            vec![],
            crate::test_support::state_subscription::read_driver(),
        )
        .with_terminal(terminal);
        let deps = subscriptions.deps();
        let _stream = deps.stream("client".into()).unwrap();
        let target = SubscriptionTarget::Terminal(surface.owner);
        deps.start_subscription("client", &target, "terminal", None)
            .await
            .unwrap();
        deps.start_subscription(
            "client",
            &SubscriptionTarget::RepositoryPaths,
            "state",
            None,
        )
        .await
        .unwrap();
        // When / Then
        subscriptions
            .terminal
            .terminal_processed("terminal", 5000)
            .unwrap();
        for id in ["unknown", "state"] {
            assert!(matches!(
                subscriptions
                    .terminal
                    .terminal_processed(id, 5000)
                    .unwrap_err()
                    .source,
                StateReadFailure::TerminalSubscriptionEnded
            ));
        }
        deps.stop_subscription("terminal").await.unwrap();
        assert!(matches!(
            subscriptions
                .terminal
                .terminal_processed("terminal", 5000)
                .unwrap_err()
                .source,
            StateReadFailure::TerminalSubscriptionEnded
        ));
    }
}
