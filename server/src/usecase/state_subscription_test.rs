use super::*;
use crate::usecase::test_helpers::state_subscription::*;

#[tokio::test]
async fn test_購読手順_開始と停止で購読状態と出力を更新する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
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
    // When
    usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    assert!(usecase.active_targets().contains(&target));
    usecase
        .stop_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    // Then
    assert!(usecase.active_targets().is_empty());
    assert_eq!(*output.initial.lock(), vec![target]);
}

#[tokio::test]
async fn test_購読手順_初期配信の失敗時にclientの対象を戻す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    output
        .fail_initial
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let usecase = StateSubscriptionUsecase::new_with_output(
        output,
        crate::test_support::state_subscription::pending_read_driver(),
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
    // When
    let result = usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await;
    // Then
    assert!(matches!(result, Err(StateReadError {
        source: StateReadFailure::Subscription(error), ..
    }) if *error == SubscriptionError::EncodingFailed));
    assert!(usecase.active_targets().is_empty());
}

#[tokio::test]
async fn test_購読手順_streamが無いと開始できない() {
    // Given
    let usecase = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        crate::test_support::state_subscription::pending_read_driver(),
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
        .start_subscription("client", &target, &FakeDelivery)
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
        crate::test_support::state_subscription::pending_read_driver(),
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

#[test]
fn test_監視後始末_停止失敗は保持して次のreconcileで止め直す() {
    let files = Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(
        Arc::new(RecordingReads {
            calls: Default::default(),
        }),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec![],
        String::new(),
    );
    let target = SubscriptionTarget::ReviewThreads("/repo".into());
    subscriptions.open_client("client".into()).unwrap();
    subscriptions.start("client", &target).unwrap();
    assert!(subscriptions.reconcile_watches().is_empty());
    assert_eq!(subscriptions.test_watches().len(), 1);
    files
        .fail_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    subscriptions.stop("client", &target).unwrap();
    assert_eq!(subscriptions.test_watches().len(), 1);
    files
        .fail_stop
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(subscriptions.reconcile_watches().is_empty());
    assert!(subscriptions.test_watches().is_empty());
    assert!(files.active.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_監視失敗_読めたrepositoryを残し対象要素だけ失敗にする() {
    use crate::usecase::fetched::Fetched;
    use crate::usecase::workspace_tree::{WorkspaceList, WorkspaceListRepository};
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(
        Arc::new(RecordingReads {
            calls: Default::default(),
        }),
        None,
        vec![],
        String::new(),
    );
    let list = WorkspaceList {
        repositories: vec![
            WorkspaceListRepository {
                path: "/failed".into(),
                worktrees: Fetched::ready(vec![]),
            },
            WorkspaceListRepository {
                path: "/healthy".into(),
                worktrees: Fetched::ready(vec![]),
            },
        ],
    };
    subscriptions
        .apply_watch_failures(
            &SubscriptionTarget::Workspaces,
            vec![(
                WatchRequirement::Git("/failed".into()),
                StateReadError::from_error(crate::usecase::watcher::UsecaseError::File(
                    "watch failed".into(),
                )),
            )],
        )
        .await;
    subscriptions
        .publish_read(
            &SubscriptionTarget::Workspaces,
            Ok(StateValue::Workspaces(list)),
            false,
        )
        .unwrap();
    let values = output.update_values.lock();
    let StateValue::Workspaces(list) = &values[0] else {
        panic!("workspaces expected")
    };
    assert!(list.repositories[0].worktrees.value.is_some());
    assert!(list.repositories[0]
        .worktrees
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("watch failed"));
    assert!(list.repositories[1].worktrees.error.is_none());
    drop(values);
    subscriptions
        .apply_watch_failures(&SubscriptionTarget::Workspaces, vec![])
        .await;
    subscriptions
        .publish_read(
            &SubscriptionTarget::Workspaces,
            Ok(StateValue::Workspaces(WorkspaceList {
                repositories: vec![WorkspaceListRepository {
                    path: "/failed".into(),
                    worktrees: Fetched::ready(vec![]),
                }],
            })),
            false,
        )
        .unwrap();
    let values = output.update_values.lock();
    let StateValue::Workspaces(list) = values.last().unwrap() else {
        panic!("workspaces expected")
    };
    assert!(list.repositories[0].worktrees.error.is_none());
}

#[tokio::test]
async fn test_購読手順_初回読取を共有し変化で再読取して最後の停止でworkerを解放する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RecordingReads {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::NodeDetail("/repo".into(), "node".into());
    usecase.open_client("first".into()).unwrap();
    usecase.open_client("second".into()).unwrap();

    // When
    usecase
        .start_subscription("first", &target, &FakeDelivery)
        .await
        .unwrap();
    usecase
        .start_subscription("second", &target, &FakeDelivery)
        .await
        .unwrap();
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
    usecase
        .stop_subscription("first", &target, &FakeDelivery)
        .await
        .unwrap();
    // Then
    assert_eq!(usecase.test_worker_count(), 1);
    // When
    usecase
        .stop_subscription("second", &target, &FakeDelivery)
        .await
        .unwrap();
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::NodeDetail("/repo".into(), "node".into());
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let waited = SubscriptionTarget::NodeDetail("/repo".into(), "one".into());
    let other = SubscriptionTarget::NodeDetail("/repo".into(), "two".into());
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &waited, &FakeDelivery)
        .await
        .unwrap();
    usecase
        .start_subscription("client", &other, &FakeDelivery)
        .await
        .unwrap();

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

async fn notify_while_reading(
    target: SubscriptionTarget,
    first: StateChangeSource,
    changes: Vec<StateChangeSource>,
) -> (Arc<GatedReads>, Arc<RecordingOutput>) {
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
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

#[tokio::test]
async fn test_購読読取_初回失敗後も登録を残す() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(FailingReads {
        fail_read: true.into(),
        fail_refresh: false.into(),
    });
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    // When
    usecase
        .start_subscription("client", &target, &FakeDelivery)
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
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
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = SubscriptionTarget::RepositoryPaths;
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
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

#[tokio::test]
async fn test_notion購読_同じ対象の2つ目の開始では取り直さない() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(GatedReads::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions
        .start_subscription("a", &target, &FakeDelivery)
        .await
        .unwrap();
    // When
    subscriptions
        .start_subscription("b", &target, &FakeDelivery)
        .await
        .unwrap();
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
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions
        .start_subscription("a", &target, &FakeDelivery)
        .await
        .unwrap();
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
async fn test_notion購読_最後のclientが閉じたらworkerを止める() {
    // Given
    let reads = Arc::new(GatedReads::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads, None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions
        .start_subscription("a", &target, &FakeDelivery)
        .await
        .unwrap();
    subscriptions
        .start_subscription("b", &target, &FakeDelivery)
        .await
        .unwrap();
    subscriptions.close_client("a");
    let before = subscriptions.test_worker_count();
    // When
    subscriptions.close_client("b");
    // Then
    assert_eq!(before, 1);
    assert_eq!(subscriptions.test_worker_count(), 0);
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
    fn acquire_external(&self, target: &SubscriptionTarget) {
        self.retained.lock().insert(target.clone());
    }
    async fn refresh_external(&self, target: &SubscriptionTarget) -> Result<(), StateReadError> {
        if self.pause.load(std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        assert!(self.retained.lock().contains(target));
        Ok(())
    }
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        Ok(StateValue::NodeDetail(None))
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
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    let other = SubscriptionTarget::NotionLabelOptions("/other".into());
    reads.retained.lock().insert(other.clone());
    subscriptions.open_client("client".into()).unwrap();
    let starting = subscriptions.clone();
    let start_target = target.clone();
    // When
    let task = tokio::spawn(async move {
        starting
            .start_subscription("client", &start_target, &FakeDelivery)
            .await
    });
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
async fn test_notion購読_別対象の監視失敗は開始を妨げず対象へ通知する() {
    // Given
    let reads = Arc::new(RetainingReads::default());
    let watcher = Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        Arc::new(crate::usecase::test_helpers::watcher::SubscriptionFiles::default()),
    ));
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
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
    let result = subscriptions
        .start_subscription("client", &target, &FakeDelivery)
        .await;
    subscriptions.close_client("watcher");
    subscriptions.close_client("client");
    // Then
    assert!(result.is_ok());
    assert_eq!(output.failures.lock().len(), 1);
    assert_eq!(
        output.failures.lock()[0].0,
        SubscriptionTarget::ReviewThreads("repo".into())
    );
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
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output,
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    let other = SubscriptionTarget::NotionLabelOptions("/other".into());
    reads.retained.lock().insert(other.clone());
    subscriptions.open_client("client".into()).unwrap();
    // When
    let result = subscriptions
        .start_subscription("client", &target, &FakeDelivery)
        .await;
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
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    subscriptions.open_client("a".into()).unwrap();
    subscriptions.open_client("b".into()).unwrap();
    subscriptions
        .start_subscription("a", &target, &FakeDelivery)
        .await
        .unwrap();
    subscriptions.close_client("b");
    // When
    let result = subscriptions
        .start_subscription("b", &target, &FakeDelivery)
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

#[tokio::test]
async fn test_外部情報の購読_lagged後のrepository通知では_notionと_issueを取り直さない() {
    // Given
    let targets = [
        notion_target(),
        SubscriptionTarget::NotionLabelOptions("/repo".into()),
        SubscriptionTarget::Issues("/repo".into()),
    ];
    let mut changes = vec![StateChangeSource::AppConfig; 200];
    changes.push(StateChangeSource::Repositories);
    let mut results = Vec::new();
    // When
    for target in targets {
        let first = match &target {
            SubscriptionTarget::Issues(path) => StateChangeSource::Issues(path.clone()),
            _ => StateChangeSource::NotionConfig("/repo".into()),
        };
        let expected = if matches!(target, SubscriptionTarget::Issues(_)) {
            1
        } else {
            2
        };
        let (reads, output) = notify_while_reading(target, first, changes.clone()).await;
        results.push((reads.external(), expected, output.updates.lock().len()));
    }
    // Then
    for (external, expected, updates) in results {
        assert_eq!(external, expected);
        assert_eq!(updates, 2);
    }
}

#[tokio::test]
async fn test_購読開始_初回取得のtaskが中断されても開始中の保有と結果を解放する() {
    // Given
    let target = notion_target();
    let reads = Arc::new(RetainingReads::default());
    reads.pause.store(true, std::sync::atomic::Ordering::SeqCst);
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        Arc::new(RecordingOutput::default()),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    subscriptions.open_client("client".into()).unwrap();
    let starting = subscriptions.clone();
    let start_target = target.clone();
    // When
    let task = tokio::spawn(async move {
        starting
            .start_subscription("client", &start_target, &FakeDelivery)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), reads.entered.notified())
        .await
        .unwrap();
    task.abort();
    let result = task.await;
    let retained = reads.retained.lock().contains(&target);
    let starting = subscriptions.starting_target.lock().clone();
    let releases = reads.releases.lock().clone();
    subscriptions.close_client("client");
    // Then
    assert!(result.unwrap_err().is_cancelled());
    assert!(!retained);
    assert!(starting.is_none());
    assert_eq!(releases, vec![target]);
}

struct InitialGatedReads {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl StateSubscriptionRead for InitialGatedReads {
    async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(StateValue::RepositoryPaths(vec![call.to_string()]))
    }
    fn repositories(&self) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test]
async fn test_購読開始_初回読取り中の変化をcontrollerへ引き継いで配信する() {
    // Given
    let reads = Arc::new(InitialGatedReads {
        entered: Default::default(),
        release: Default::default(),
        calls: Default::default(),
    });
    let output = Arc::new(RecordingOutput::default());
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    usecase.open_client("client".into()).unwrap();
    let started = tokio::spawn({
        let usecase = usecase.clone();
        async move {
            usecase
                .start_subscription(
                    "client",
                    &SubscriptionTarget::RepositoryPaths,
                    &FakeDelivery,
                )
                .await
                .unwrap();
        }
    });
    reads.entered.notified().await;
    // When
    usecase.notify(StateChangeSource::Repositories);
    reads.release.notify_one();
    started.await.unwrap();
    output.updated.notified().await;
    // Then
    assert_eq!(
        *output.initial_values.lock(),
        vec![StateValue::RepositoryPaths(vec!["0".into()])]
    );
    assert_eq!(
        *output.update_values.lock(),
        vec![StateValue::RepositoryPaths(vec!["1".into()])]
    );
    usecase.close_client("client");
}

#[tokio::test]
async fn test_購読開始_駆動部が終了したら登録を戻して失敗を返す() {
    // Given
    let (driver, requests) = tokio::sync::mpsc::unbounded_channel();
    drop(requests);
    let usecase =
        StateSubscriptionUsecase::new_with_output(Arc::new(RecordingOutput::default()), driver)
            .with_reads(
                Arc::new(RecordingReads {
                    calls: Default::default(),
                }),
                None,
                vec![],
                String::new(),
            );
    usecase.open_client("client".into()).unwrap();
    // When
    let error = usecase
        .start_subscription(
            "client",
            &SubscriptionTarget::RepositoryPaths,
            &FakeDelivery,
        )
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error.source, StateReadFailure::Subscription(error)
        if *error == SubscriptionError::StreamEnded));
    assert!(usecase.active_targets().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
}

#[tokio::test]
async fn test_購読共有_同じclientの片方を停止しても取得とworkerを保持する() {
    // Given
    let output = Arc::new(RecordingOutput::default());
    let reads = Arc::new(RetainingReads::default());
    let usecase = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(reads.clone(), None, vec![], String::new());
    let target = notion_target();
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    usecase
        .start_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    assert_eq!(output.initial.lock().len(), 1);
    // When
    usecase
        .stop_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    // Then
    assert!(usecase.active_targets().contains(&target));
    assert!(reads.retained.lock().contains(&target));
    assert!(reads.releases.lock().is_empty());
    assert_eq!(usecase.test_worker_count(), 1);
    usecase
        .stop_subscription("client", &target, &FakeDelivery)
        .await
        .unwrap();
    assert!(!usecase.active_targets().contains(&target));
    assert!(!reads.retained.lock().contains(&target));
    assert_eq!(usecase.test_worker_count(), 0);
}

#[tokio::test]
async fn test_購読連携_usecaseが配信開始と失敗時の読取停止を行う() {
    // Given
    struct Delivery {
        usecase: StateSubscriptionUsecase,
        output: Arc<RecordingOutput>,
        fail: bool,
        calls: Mutex<Vec<&'static str>>,
    }
    impl StateSubscriptionDelivery for Delivery {
        fn start(&self) -> Result<Option<usize>, StateReadError> {
            assert!(!self.usecase.active_targets().is_empty());
            assert_eq!(self.output.initial.lock().len(), 1);
            self.calls.lock().push("start");
            if self.fail {
                Err(StateReadError::from_error(SubscriptionError::UnknownTarget))
            } else {
                Ok(None)
            }
        }
        fn claim(&self) -> bool {
            self.calls.lock().push("claim");
            true
        }
        fn finish(
            &self,
            active: &std::collections::HashSet<SubscriptionTarget>,
        ) -> Result<(), SubscriptionError> {
            assert!(active.is_empty());
            self.calls.lock().push("finish");
            Ok(())
        }
    }
    for fail in [false, true] {
        let output = Arc::new(RecordingOutput::default());
        let usecase = StateSubscriptionUsecase::new_with_output(
            output.clone(),
            crate::test_support::state_subscription::read_driver(),
        )
        .with_reads(
            Arc::new(RecordingReads {
                calls: Default::default(),
            }),
            None,
            vec![],
            String::new(),
        );
        usecase.open_client("client".into()).unwrap();
        let delivery = Delivery {
            usecase: usecase.clone(),
            output,
            fail,
            calls: Default::default(),
        };
        let target = SubscriptionTarget::RepositoryPaths;
        // When
        let result = usecase
            .start_subscription("client", &target, &delivery)
            .await;
        // Then
        if fail {
            assert!(
                matches!(result, Err(StateReadError { source: StateReadFailure::Subscription(error), .. }) if *error == SubscriptionError::UnknownTarget)
            );
            assert_eq!(*delivery.calls.lock(), vec!["start"]);
        } else {
            result.unwrap();
            usecase
                .stop_subscription("client", &target, &delivery)
                .await
                .unwrap();
            assert_eq!(*delivery.calls.lock(), vec!["start", "claim", "finish"]);
        }
        assert!(usecase.active_targets().is_empty());
        assert_eq!(usecase.test_worker_count(), 0);
    }
}

struct WatchFailureReads(crate::usecase::workspace_tree::WorkspaceList);

#[async_trait::async_trait]
impl StateSubscriptionRead for WatchFailureReads {
    async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
        Ok(match target {
            SubscriptionTarget::Workspaces => StateValue::Workspaces(self.0.clone()),
            SubscriptionTarget::Workflows => StateValue::Workflows(vec![]),
            _ => StateValue::CurrentBranch("main".into()),
        })
    }
    fn repositories(&self) -> Vec<String> {
        vec!["/failed".into(), "/healthy".into()]
    }
    fn workflows_dir(&self) -> String {
        "/workflows".into()
    }
}

#[tokio::test]
async fn test_監視失敗_別購読で検出してもworkspacesの要素へ届け成功で解く() {
    use crate::usecase::fetched::Fetched;
    use crate::usecase::workspace_tree::{
        WorkspaceList, WorkspaceListRepository, WorkspaceListWorktree,
    };
    // Given
    let list = WorkspaceList {
        repositories: vec![
            WorkspaceListRepository {
                path: "/failed".into(),
                worktrees: Fetched::ready(vec![WorkspaceListWorktree {
                    tracking: Fetched::ready(None),
                    worktree: crate::domain::repository::Worktree::being_deleted(
                        "/failed/linked",
                        "branch".into(),
                    ),
                    deleting: false,
                    dirty_count: Fetched::ready(42),
                    merged: false,
                    pull_request: None,
                    pull_request_loaded: false,
                    tree: Fetched::default(),
                    pull_request_error: None,
                }]),
            },
            WorkspaceListRepository {
                path: "/healthy".into(),
                worktrees: Fetched::ready(vec![]),
            },
        ],
    };
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(
        Arc::new(WatchFailureReads(list.clone())),
        None,
        vec![],
        String::new(),
    );
    let branch = SubscriptionTarget::CurrentBranch("/failed/linked".into());
    subscriptions.open_client("client".into()).unwrap();
    for target in [
        &SubscriptionTarget::Workspaces,
        &branch,
        &SubscriptionTarget::Workflows,
    ] {
        subscriptions.start("client", target).unwrap();
    }
    // When
    subscriptions
        .apply_watch_failures(
            &branch,
            vec![
                (
                    WatchRequirement::Git("/failed".into()),
                    StateReadError::from_error(crate::usecase::watcher::UsecaseError::File(
                        "root failed".into(),
                    )),
                ),
                (
                    WatchRequirement::Git("/failed/linked".into()),
                    StateReadError::from_error(crate::usecase::watcher::UsecaseError::File(
                        "linked failed".into(),
                    )),
                ),
            ],
        )
        .await;
    subscriptions
        .publish_read(&branch, Ok(StateValue::CurrentBranch("main".into())), false)
        .unwrap();
    assert!(output
        .failures
        .lock()
        .iter()
        .any(|(target, _)| target == &branch));
    // Then
    let values = output.update_values.lock();
    let StateValue::Workspaces(failed) = &values[0] else {
        panic!("workspace list must remain a value")
    };
    assert_eq!(failed.repositories[1], list.repositories[1]);
    assert!(failed.repositories[0].worktrees.error.is_some());
    let linked = &failed.repositories[0].worktrees.value.as_ref().unwrap()[0];
    assert_eq!(linked.dirty_count.value, Some(42));
    assert!(linked.dirty_count.error.is_some());
    assert!(linked.tree.error.is_some());
    drop(values);
    assert!(output
        .failures
        .lock()
        .iter()
        .all(|(target, _)| *target != SubscriptionTarget::Workspaces));
    subscriptions.apply_watch_failures(&branch, vec![]).await;
    assert_eq!(
        output.update_values.lock().last(),
        Some(&StateValue::Workspaces(list.clone()))
    );
    let files =
        WatchRequirement::Files("/workflows".into(), StateChangeSource::WorkflowDefinitions);
    subscriptions
        .apply_watch_failures(
            &branch,
            vec![(
                files,
                StateReadError::from_error(crate::usecase::watcher::UsecaseError::File(
                    "definitions failed".into(),
                )),
            )],
        )
        .await;
    assert_eq!(
        output.failures.lock().last().unwrap().0,
        SubscriptionTarget::Workflows
    );
    subscriptions.apply_watch_failures(&branch, vec![]).await;
    assert_eq!(
        output.update_values.lock().last(),
        Some(&StateValue::Workflows(vec![]))
    );
}

#[tokio::test]
async fn test_監視対象読み取り失敗_reconcileから該当repositoryだけに載せる() {
    use crate::usecase::fetched::Fetched;
    use crate::usecase::workspace_tree::{WorkspaceList, WorkspaceListRepository};
    struct Reads(WorkspaceList);
    #[async_trait::async_trait]
    impl StateSubscriptionRead for Reads {
        async fn read(&self, _: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            Ok(StateValue::Workspaces(self.0.clone()))
        }
        fn repositories(&self) -> Vec<String> {
            vec!["/failed".into(), "/healthy".into()]
        }
        fn watch_paths(
            &self,
        ) -> (
            Vec<String>,
            Vec<(String, crate::domain::failure::WorkFailure)>,
        ) {
            (
                vec![],
                vec![(
                    "/failed".into(),
                    crate::domain::failure::WorkFailure::from_error(
                        &crate::usecase::watcher::UsecaseError::File("root read failed".into()),
                    ),
                )],
            )
        }
    }
    struct Files;
    impl crate::domain::repository::file_watcher::FileWatchGateway for Files {
        fn start_tree(
            &self,
            _: &str,
            _: crate::domain::repository::file_watcher::WatchChangeHandler,
        ) -> Result<u64, String> {
            unreachable!()
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            unreachable!()
        }
    }
    let list = WorkspaceList {
        repositories: vec![
            WorkspaceListRepository {
                path: "/failed".into(),
                worktrees: Fetched::ready(vec![]),
            },
            WorkspaceListRepository {
                path: "/healthy".into(),
                worktrees: Fetched::ready(vec![]),
            },
        ],
    };
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
    )
    .with_reads(
        Arc::new(Reads(list.clone())),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            Arc::new(Files),
        ))),
        vec![],
        String::new(),
    );
    subscriptions.open_client("client".into()).unwrap();
    subscriptions
        .start("client", &SubscriptionTarget::Workspaces)
        .unwrap();
    let mut changes = subscriptions.changes.subscribe();
    let (_sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
    // When
    subscriptions
        .refresh_read(
            &SubscriptionTarget::Workspaces,
            ReadSignal::Lagged,
            &mut changes,
            &mut waiting,
        )
        .await;
    // Then
    let updates = output.update_values.lock();
    let StateValue::Workspaces(failed) = &updates[0] else {
        panic!("workspace value required")
    };
    assert_eq!(failed.repositories[1], list.repositories[1]);
    assert_eq!(
        failed.repositories[0].worktrees.value,
        list.repositories[0].worktrees.value
    );
    assert!(failed.repositories[0]
        .worktrees
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("root read failed"));
}

#[tokio::test]
async fn test_ファイル監視_読取中の失敗は張り直し成功まで解除しない() {
    use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[derive(Default)]
    struct Files {
        callbacks: Mutex<Vec<WatchChangeHandler>>,
        attempts: AtomicUsize,
        fail_start: AtomicBool,
    }
    impl FileWatchGateway for Files {
        fn start_tree(&self, _: &str, callback: WatchChangeHandler) -> Result<u64, String> {
            let id = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_start.load(Ordering::SeqCst) {
                return Err("restart failed".into());
            }
            self.callbacks.lock().push(callback);
            Ok(id as u64)
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    struct Reads {
        block: AtomicBool,
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl StateSubscriptionRead for Reads {
        async fn read(&self, target: &SubscriptionTarget) -> Result<StateValue, StateReadError> {
            match target {
                SubscriptionTarget::Workflows => {
                    if self.block.swap(false, Ordering::SeqCst) {
                        self.started.notify_one();
                        self.release.notified().await;
                    }
                    Ok(StateValue::Workflows(vec![]))
                }
                SubscriptionTarget::WorkflowSource(_) => {
                    Ok(StateValue::WorkflowSource(Some("source".into())))
                }
                _ => panic!("unexpected target"),
            }
        }
        fn repositories(&self) -> Vec<String> {
            vec![]
        }
        fn workflows_dir(&self) -> String {
            "/workflows".into()
        }
    }
    // Given
    let files = Arc::new(Files::default());
    let reads = Arc::new(Reads {
        block: AtomicBool::new(true),
        started: Default::default(),
        release: Default::default(),
    });
    let output = Arc::new(RecordingOutput::default());
    let subscriptions = StateSubscriptionUsecase::new_with_output(
        output.clone(),
        crate::test_support::state_subscription::pending_read_driver(),
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
    subscriptions.open_client("client".into()).unwrap();
    let target = SubscriptionTarget::Workflows;
    let source = SubscriptionTarget::WorkflowSource("workflow".into());
    subscriptions.start("client", &target).unwrap();
    subscriptions.start("client", &source).unwrap();
    assert!(subscriptions.reconcile_watches().is_empty());
    let refreshing = subscriptions.clone();
    let pending = tokio::spawn(async move {
        let mut changes = refreshing.changes.subscribe();
        let (_sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
        refreshing
            .refresh_read(
                &SubscriptionTarget::Workflows,
                ReadSignal::Lagged,
                &mut changes,
                &mut waiting,
            )
            .await;
    });
    // When
    tokio::time::timeout(std::time::Duration::from_secs(1), reads.started.notified())
        .await
        .unwrap();
    files.callbacks.lock()[0].clone()(Err("runtime watch failed".into()));
    reads.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), pending)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(files.attempts.load(Ordering::SeqCst), 1);
    assert!(subscriptions.watches.lock().is_empty());
    assert_eq!(subscriptions.watch_failures.lock().len(), 1);
    assert!(output.update_values.lock().is_empty());
    for affected in [&target, &source] {
        assert!(output.failures.lock().iter().any(|(reported, message)| {
            reported == affected && message.contains("runtime watch failed")
        }));
    }
    // When
    files.fail_start.store(true, Ordering::SeqCst);
    let mut changes = subscriptions.changes.subscribe();
    let (_sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
    subscriptions
        .refresh_read(&target, ReadSignal::Periodic, &mut changes, &mut waiting)
        .await;
    // Then
    assert_eq!(files.attempts.load(Ordering::SeqCst), 2);
    assert!(subscriptions.watches.lock().is_empty());
    assert_eq!(subscriptions.watch_failures.lock().len(), 1);
    assert!(output.update_values.lock().is_empty());
    for affected in [&target, &source] {
        assert!(output.failures.lock().iter().any(|(reported, message)| {
            reported == affected && message.contains("restart failed")
        }));
    }
    // When
    files.fail_start.store(false, Ordering::SeqCst);
    subscriptions
        .refresh_read(&target, ReadSignal::Periodic, &mut changes, &mut waiting)
        .await;
    // Then
    assert_eq!(files.attempts.load(Ordering::SeqCst), 3);
    assert_eq!(subscriptions.watches.lock().len(), 1);
    assert!(subscriptions.watch_failures.lock().is_empty());
    assert_eq!(*output.updates.lock(), vec![source.clone(), target.clone()]);
    assert_eq!(
        *output.update_values.lock(),
        vec![
            StateValue::WorkflowSource(Some("source".into())),
            StateValue::Workflows(vec![]),
        ]
    );
    // When
    files.callbacks.lock().last().unwrap().clone()(Err("runtime failed again".into()));
    subscriptions.stop("client", &source).unwrap();
    subscriptions.stop("client", &target).unwrap();
    subscriptions.apply_watch_failures(&target, vec![]).await;
    // Then
    assert!(subscriptions.watch_failures.lock().is_empty());
}

#[tokio::test]
async fn test_ファイル監視_稼働中の失敗を配信し張り直しで値へ戻す() {
    use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
    #[derive(Default)]
    struct Files {
        callbacks: Mutex<Vec<WatchChangeHandler>>,
        stopped: Mutex<Vec<u64>>,
        fail_stop: std::sync::atomic::AtomicBool,
    }
    impl FileWatchGateway for Files {
        fn start_tree(&self, _: &str, callback: WatchChangeHandler) -> Result<u64, String> {
            let mut callbacks = self.callbacks.lock();
            callbacks.push(callback);
            Ok(callbacks.len() as u64)
        }
        fn stop(&self, id: u64) -> Result<(), String> {
            self.stopped.lock().push(id);
            if self
                .fail_stop
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                Err("stop failed".into())
            } else {
                Ok(())
            }
        }
    }
    for fail_stop in [false, true] {
        let files = Arc::new(Files::default());
        files
            .fail_stop
            .store(fail_stop, std::sync::atomic::Ordering::SeqCst);
        let output = Arc::new(RecordingOutput::default());
        let subscriptions = StateSubscriptionUsecase::new_with_output(
            output.clone(),
            crate::test_support::state_subscription::pending_read_driver(),
        )
        .with_reads(
            Arc::new(WatchFailureReads(
                crate::usecase::workspace_tree::WorkspaceList {
                    repositories: vec![],
                },
            )),
            Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                None,
                files.clone(),
            ))),
            vec![],
            String::new(),
        );
        subscriptions.open_client("client".into()).unwrap();
        let target = SubscriptionTarget::Workflows;
        subscriptions.start("client", &target).unwrap();
        assert!(subscriptions.reconcile_watches().is_empty());
        let callback = files.callbacks.lock()[0].clone();
        callback(Err("runtime watch failed".into()));
        assert_eq!(
            output.failures.lock().last().unwrap(),
            &(
                target.clone(),
                crate::domain::failure::WorkFailure::from_error(
                    &crate::usecase::watcher::UsecaseError::File("runtime watch failed".into())
                )
                .message
            )
        );
        assert!(subscriptions.watches.lock().is_empty());
        assert_eq!(*files.stopped.lock(), vec![1]);
        assert_eq!(subscriptions.watch_failures.lock().len(), 1);
        let first_message = output.failures.lock().last().unwrap().1.clone();
        subscriptions
            .publish_read(&target, Ok(StateValue::Workflows(vec![])), false)
            .unwrap();
        assert_eq!(output.failures.lock().last().unwrap().1, first_message);
        let mut changes = subscriptions.changes.subscribe();
        let (_sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
        subscriptions
            .refresh_read(&target, ReadSignal::Periodic, &mut changes, &mut waiting)
            .await;
        assert_eq!(files.callbacks.lock().len(), 2);
        assert!(subscriptions.watch_failures.lock().is_empty());
        assert_eq!(
            output.update_values.lock().last(),
            Some(&StateValue::Workflows(vec![]))
        );
        let failure_count = output.failures.lock().len();
        callback(Err("late old failure".into()));
        assert_eq!(subscriptions.watches.lock().len(), 1);
        assert!(subscriptions.watch_failures.lock().is_empty());
        assert_eq!(output.failures.lock().len(), failure_count);
        assert!(subscriptions.pending_watch_stops.lock().is_empty());
        assert_eq!(
            *files.stopped.lock(),
            if fail_stop { vec![1, 1] } else { vec![1] }
        );
    }
}

#[test]
fn test_ファイル監視_失敗と張り直しの配信順序を排他で保つ() {
    use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Default)]
    struct Files(Mutex<Vec<WatchChangeHandler>>);
    impl FileWatchGateway for Files {
        fn start_tree(&self, _: &str, callback: WatchChangeHandler) -> Result<u64, String> {
            let mut callbacks = self.0.lock();
            callbacks.push(callback);
            Ok(callbacks.len() as u64)
        }
        fn stop(&self, _: u64) -> Result<(), String> {
            Ok(())
        }
    }
    struct Output {
        publications: Mutex<Vec<bool>>,
        gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    }
    impl Output {
        fn record(&self, failed: bool) -> Result<(), SubscriptionError> {
            let gate = self.gate.lock().take();
            if let Some((started, release)) = gate {
                started.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.publications.lock().push(failed);
            Ok(())
        }
    }
    impl StateSubscriptionOutput for Output {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn publish_failure(
            &self,
            _: &SubscriptionTarget,
            _: StateReadError,
        ) -> Result<(), SubscriptionError> {
            self.record(true)
        }
        fn publish_initial(
            &self,
            _: &SubscriptionTarget,
            _: StateValue,
        ) -> Result<(), SubscriptionError> {
            self.record(false)
        }
        fn publish(
            &self,
            _: &SubscriptionTarget,
            _: StateValue,
            _: Option<StateValue>,
        ) -> Result<(), SubscriptionError> {
            self.record(false)
        }
    }

    for failure_first in [true, false] {
        // Given
        let (started, started_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let output = Arc::new(Output {
            publications: Default::default(),
            gate: Mutex::new(Some((started, release_rx))),
        });
        let files = Arc::new(Files::default());
        let subscriptions = StateSubscriptionUsecase::new_with_output(
            output.clone(),
            crate::test_support::state_subscription::pending_read_driver(),
        )
        .with_reads(
            Arc::new(WatchFailureReads(
                crate::usecase::workspace_tree::WorkspaceList {
                    repositories: vec![],
                },
            )),
            Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                None,
                files.clone(),
            ))),
            vec![],
            String::new(),
        );
        let target = SubscriptionTarget::Workflows;
        subscriptions.open_client("client".into()).unwrap();
        subscriptions.start("client", &target).unwrap();
        assert!(subscriptions.reconcile_watches().is_empty());
        let callback = files.0.lock()[0].clone();

        // When
        let first_subscriptions = subscriptions.clone();
        let first_callback = callback.clone();
        let first = std::thread::spawn(move || {
            if failure_first {
                first_callback(Err("runtime watch failed".into()));
            } else {
                first_subscriptions
                    .publish_read(
                        &SubscriptionTarget::Workflows,
                        Ok(StateValue::Workflows(vec![])),
                        false,
                    )
                    .unwrap();
            }
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let excluded = subscriptions.watch_failures.try_lock().is_none();
        let (attempted, attempted_rx) = mpsc::channel();
        let second_subscriptions = subscriptions.clone();
        let second = std::thread::spawn(move || {
            attempted.send(()).unwrap();
            if failure_first {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let mut changes = second_subscriptions.changes.subscribe();
                        let (_sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
                        second_subscriptions
                            .refresh_read(
                                &SubscriptionTarget::Workflows,
                                ReadSignal::Periodic,
                                &mut changes,
                                &mut waiting,
                            )
                            .await;
                    });
            } else {
                callback(Err("runtime watch failed".into()));
            }
        });
        attempted_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();

        // Then
        assert!(excluded, "失敗の参照から配信の完了まで排他を保つ");
        assert_eq!(
            *output.publications.lock(),
            vec![failure_first, !failure_first]
        );
        assert_eq!(
            subscriptions.watch_failures.lock().is_empty(),
            failure_first
        );
        assert_eq!(
            subscriptions.watches.lock().len(),
            usize::from(failure_first)
        );
    }
}
