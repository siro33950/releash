use super::*;

#[tokio::test(start_paused = true)]
async fn test_購読_配信と定期印と終了時の解放() {
    // Given
    let usecase = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    // When
    usecase.start("client", REPO_PATHS, None).unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == StateValue::RepositoryPaths(vec!["/repo".into()]))
    );
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    usecase
        .publisher()
        .publish(
            REPO_PATHS,
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
        usecase.start("client", REPO_PATHS, None),
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
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
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

    assert_eq!(
        usecase.start("waiting", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    assert_eq!(
        usecase
            .publisher()
            .publish("missing", StateValue::RepositoryPaths(vec![]), None),
        Err(SubscriptionError::UnknownTarget)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    usecase.start("waiting", REPO_PATHS, None).unwrap();

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
            REPO_PATHS,
            StateValue::RepositoryPaths(vec!["/next".into()]),
            None,
        )
        .unwrap();

    // Then
    assert!(flag.0.load(Ordering::SeqCst));
    assert!(
        matches!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Change(Version { sequence: 1, .. }, Delivery::Full, value)))) if *value == StateValue::RepositoryPaths(vec!["/next".into()]))
    );

    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    flag.0.store(false, Ordering::SeqCst);
    assert_eq!(
        usecase.stop("missing", REPO_PATHS),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    usecase.stop("waiting", REPO_PATHS).unwrap();

    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    usecase
        .publisher()
        .publish(REPO_PATHS, StateValue::RepositoryPaths(vec![]), None)
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
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(match target {
            crate::domain::state_subscription::SubscriptionTarget::SessionNode(_, _) => {
                StateValue::SessionNode(Some(self.value.lock().clone()))
            }
            crate::domain::state_subscription::SubscriptionTarget::SessionHistory(_, _) => {
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
        _: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
async fn test_引数付き購読_対象の変更だけを読み直して配信し終了でworkerを解放する() {
    use crate::domain::state_subscription::{StateChangeSource, SubscriptionTarget};
    // Given
    let reads = Arc::new(FakeReads {
        value: Mutex::new("node-1".into()),
        calls: Default::default(),
    });
    let mut usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    usecase.reads = Some(reads.clone());
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::SessionNode("/repo".into(), "session".into()).to_string();
    // When
    usecase.start_read("client", &target, None).await.unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == StateValue::SessionNode(Some("node-1".into())))
    );
    stream.next().await;
    *reads.value.lock() = "node-2".into();
    usecase
        .publisher()
        .invalidate(StateChangeSource::Worktree("/repo".into()));
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if *value == StateValue::SessionNode(Some("node-2".into())))
    );
    drop(stream);
    assert!(usecase.workers.lock().is_empty());
    assert!(usecase.publisher.state.lock().active_targets().is_empty());
}

#[tokio::test(start_paused = true)]
async fn test_外部情報_購読者がいる間だけcache_ttlで取得する() {
    use crate::domain::state_subscription::SubscriptionTarget;
    use std::sync::atomic::Ordering;
    // Given
    let reads = Arc::new(FakeReads {
        value: Mutex::new(String::new()),
        calls: Default::default(),
    });
    let mut usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    usecase.reads = Some(reads.clone());
    let stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::Issues("/repo".into()).to_string();
    usecase.start_read("client", &target, None).await.unwrap();
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
    use crate::domain::state_subscription::SubscriptionTarget;
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let mut usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    usecase.reads = Some(Arc::new(FakeReads {
        value: Mutex::new(String::new()),
        calls: Default::default(),
    }));
    usecase.watchers = Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    )));
    usecase.history_paths = vec!["/claude".into(), "/codex".into()];
    let first = usecase.open("first".into()).unwrap();
    let second = usecase.open("second".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
    usecase.start_read("first", &target, None).await.unwrap();
    usecase.start_read("second", &target, None).await.unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 2);
    assert_eq!(usecase.workers.lock().len(), 1);
    usecase.stop("first", &target).unwrap();
    let expanded = SubscriptionTarget::SessionHistory("/repo".into(), 40).to_string();
    usecase.start_read("first", &expanded, None).await.unwrap();
    assert_eq!(files.active.lock().unwrap().len(), 2);
    drop(second);
    assert_eq!(files.active.lock().unwrap().len(), 2);
    drop(first);
    assert!(files.active.lock().unwrap().is_empty());
    assert!(usecase.workers.lock().is_empty());
}

#[tokio::test]
async fn test_workspaces購読_最後の停止と切断で実際のgit監視を解放する() {
    use crate::domain::state_subscription::WatchRequirement;
    for disconnect in [false, true] {
        // Given
        let fixture = StateReadsFixture::new();
        let repository = fixture.reads.repository_state.clone();
        let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
        let mut usecase = fixture.subscriptions.clone();
        usecase.watchers = Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            Some(repository.clone()),
            files.clone(),
        )));
        let first = usecase.open("first".into()).unwrap();
        let second = usecase.open("second".into()).unwrap();
        usecase
            .start_read("first", "workspaces", None)
            .await
            .unwrap();
        usecase
            .start_read("second", "workspaces", None)
            .await
            .unwrap();
        let requirement = WatchRequirement::Git(fixture.path.clone());
        let id = usecase.watches.lock()[&requirement];
        let snapshot = repository.get_snapshot(&fixture.path).unwrap();
        assert_eq!(usecase.watches.lock().len(), 1);
        // When / Then
        usecase.stop_read("first", "workspaces").await.unwrap();
        assert_eq!(usecase.watches.lock()[&requirement], id);
        assert!(Arc::ptr_eq(
            &snapshot,
            &repository.get_snapshot(&fixture.path).unwrap()
        ));
        if !disconnect {
            usecase.stop_read("second", "workspaces").await.unwrap();
        }
        drop(second);
        assert!(usecase.watches.lock().is_empty());
        assert!(usecase.workers.lock().is_empty());
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
    use crate::domain::state_subscription::SubscriptionTarget;
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let mut usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    usecase.reads = Some(Arc::new(FakeReads {
        value: Mutex::new(String::new()),
        calls: Default::default(),
    }));
    usecase.watchers = Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        files.clone(),
    )));
    usecase.history_paths = vec!["/missing".into()];
    let _stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20).to_string();
    assert!(usecase.start_read("client", &target, None).await.is_err());
    assert!(usecase.publisher.state.lock().active_targets().is_empty());
    usecase.history_paths = vec!["/history".into()];
    usecase.start_read("client", &target, None).await.unwrap();
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
        _: &crate::domain::state_subscription::SubscriptionTarget,
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
        _: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}
#[tokio::test]
async fn test_購読停止_初回読取中の停止要求でも監視とworkerを残さない() {
    use crate::domain::state_subscription::SubscriptionTarget;
    let reads = Arc::new(BlockedReads {
        entered: Default::default(),
        release: Default::default(),
    });
    let files = Arc::new(crate::usecase::watcher::watcher_tests::SubscriptionFiles::default());
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_reads(
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
        async move { usecase.start_read("client", &target, None).await }
    });
    reads.entered.notified().await;
    let stopped = tokio::spawn({
        let usecase = usecase.clone();
        async move { usecase.stop_read("client", &target).await }
    });
    tokio::task::yield_now().await;
    assert!(!stopped.is_finished());
    reads.release.notify_one();
    started.await.unwrap().unwrap();
    stopped.await.unwrap().unwrap();
    assert!(files.active.lock().unwrap().is_empty());
    assert!(usecase.workers.lock().is_empty());
    assert!(usecase.publisher.state.lock().active_targets().is_empty());
}

struct NullableReads;
#[async_trait::async_trait]
impl StateSubscriptionRead for NullableReads {
    async fn read(
        &self,
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        Ok(match target {
            crate::domain::state_subscription::SubscriptionTarget::AgentSession(_) => {
                StateValue::AgentSession(None)
            }
            crate::domain::state_subscription::SubscriptionTarget::NodeDetail(_, _) => {
                StateValue::NodeDetail(None)
            }
            _ => panic!("unexpected target"),
        })
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

#[tokio::test]
async fn test_不在対象_初回からnullable_snapshotとして配信する() {
    use crate::domain::state_subscription::SubscriptionTarget;
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_reads(Arc::new(NullableReads), None, vec![]);
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
        usecase
            .start_read("client", &target.to_string(), None)
            .await
            .unwrap();
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == expected)
        );
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_購読開始と切断_同じ対象の最終購読者が切断しても再開できる() {
    use crate::domain::state_subscription::SubscriptionTarget;
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_reads(Arc::new(NullableReads), None, vec![]);
    let target = SubscriptionTarget::AgentSession("missing".into()).to_string();
    for index in 0..100 {
        let old_id = format!("old-{index}");
        let next_id = format!("next-{index}");
        let old = usecase.open(old_id.clone()).unwrap();
        usecase.start_read(&old_id, &target, None).await.unwrap();
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
        usecase.start_read(&next_id, &target, None).await.unwrap();
        closed.await.unwrap();
        assert!(
            matches!(next.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == StateValue::AgentSession(None))
        );
        drop(next);
        assert!(usecase.workers.lock().is_empty());
    }
}

#[derive(Default)]
struct ExternalReads {
    issues: std::sync::atomic::AtomicU64,
    prs: std::sync::atomic::AtomicU64,
}
impl ExternalReads {
    fn value(&self, target: &crate::domain::state_subscription::SubscriptionTarget) -> StateValue {
        use crate::usecase::workspace_tree::{
            WorkspaceBranchDto, WorkspaceListSnapshotDto, WorkspaceListStatusDto,
            WorkspaceRepositoryListDto,
        };
        use std::sync::atomic::Ordering;
        if matches!(
            target,
            crate::domain::state_subscription::SubscriptionTarget::Issues(_)
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
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<StateValue, StateReadError> {
        Ok(self.value(target))
    }
    async fn refresh_external(
        &self,
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<(), StateReadError> {
        if matches!(
            target,
            crate::domain::state_subscription::SubscriptionTarget::Issues(_)
        ) {
            self.issues
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
        self.prs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn repositories(&self) -> Vec<String> {
        vec!["/repo".into()]
    }
}

#[tokio::test(start_paused = true)]
async fn test_外部情報ttl_issueとprを取得し更新値を配信して停止後は取得しない() {
    use crate::domain::state_subscription::SubscriptionTarget;
    use std::sync::atomic::Ordering;
    for target in [
        SubscriptionTarget::Issues("/repo".into()),
        SubscriptionTarget::Workspaces,
    ] {
        let reads = Arc::new(ExternalReads::default());
        let usecase = StateSubscriptionUsecase::new(
            vec![],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        )
        .with_reads(reads.clone(), None, vec![]);
        let mut stream = Box::pin(usecase.open("client".into()).unwrap());
        stream.next().await;
        usecase
            .start_read("client", &target.to_string(), None)
            .await
            .unwrap();
        let count = if matches!(target, SubscriptionTarget::Issues(_)) {
            &reads.issues
        } else {
            &reads.prs
        };
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == reads.value(&target))
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
        assert_eq!(*changed, reads.value(&target));
        drop(stream);
        tokio::time::advance(std::time::Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn test_初回読取中の切断_開始失敗後に対象の鍵もworkerも残さない() {
    use crate::domain::state_subscription::SubscriptionTarget;
    // Given
    let reads = Arc::new(BlockedReads {
        entered: Default::default(),
        release: Default::default(),
    });
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_reads(reads.clone(), None, vec![]);
    let stream = usecase.open("client".into()).unwrap();
    let target = SubscriptionTarget::SessionHistory("/repo".into(), 20);
    let started = tokio::spawn({
        let usecase = usecase.clone();
        let raw = target.to_string();
        async move { usecase.start_read("client", &raw, None).await }
    });
    reads.entered.notified().await;
    // When
    drop(stream);
    reads.release.notify_one();
    // Then
    assert_eq!(
        started.await.unwrap().unwrap_err().kind,
        crate::domain::failure::FailureKind::Missing
    );
    assert!(!usecase.publisher.state.lock().registered(&target));
    assert!(usecase
        .publisher
        .state
        .lock()
        .registered(&SubscriptionTarget::RepositoryPaths));
    assert!(usecase.workers.lock().is_empty());
}

struct DisconnectingFiles {
    publisher: StateSubscriptionPublisher,
}
impl crate::domain::repository::file_watcher::FileWatchGateway for DisconnectingFiles {
    fn start(&self, _: &str) -> Result<u64, String> {
        self.publisher.state.lock().close("client");
        Ok(1)
    }
    fn stop(&self, _: u64) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn test_snapshot登録後の切断_開始失敗で対象の鍵を解放する() {
    use crate::domain::state_subscription::SubscriptionTarget;
    // Given
    let mut usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let files = Arc::new(DisconnectingFiles {
        publisher: usecase.publisher(),
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
    let error = usecase
        .start_read("client", &target.to_string(), None)
        .await
        .unwrap_err();
    // Then
    assert_eq!(error.message, "StreamEnded");
    assert!(!usecase.publisher.state.lock().registered(&target));
    assert!(usecase
        .publisher
        .state
        .lock()
        .registered(&SubscriptionTarget::RepositoryPaths));
    assert!(usecase.workers.lock().is_empty());
}
