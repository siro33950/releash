use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Runtime(tokio::time::Instant);
#[async_trait::async_trait]
impl WorkQueueRuntime for Runtime {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
    fn timestamp_ms(&self) -> u64 {
        self.now().as_millis() as u64
    }
    fn jitter(&self) -> f64 {
        1.0
    }
    fn spawn(&self, task: Task) {
        tokio::spawn(task);
    }
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
    async fn attempt(&self, attempt: Attempt<'_>) -> Result<Option<Duration>, WorkFailure> {
        tokio::time::timeout(Duration::from_secs(20), attempt)
            .await
            .unwrap_or_else(|_| {
                Err(WorkFailure {
                    kind: FailureKind::Expired,
                    message: "deadline".into(),
                })
            })
    }
}
pub(crate) fn queue() -> Arc<WorkQueueUsecase> {
    WorkQueueUsecase::new(Arc::new(Runtime(tokio::time::Instant::now())))
}

#[tokio::test(start_paused = true)]
async fn test_作業列_分類による再試行と失敗集約を同じ経路で行う() {
    // Given
    for kind in [
        FailureKind::Temporary,
        FailureKind::RestartRequired,
        FailureKind::Internal,
        FailureKind::Cancelled,
        FailureKind::StateRequired,
    ] {
        let queue = queue();
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let key = WorkKey::new("repository_scan", "/repo");
        // When
        let result = queue
            .execute(key, RetryBackoff::ITEM, move |action| {
                let call = seen.fetch_add(1, Ordering::SeqCst);
                async move {
                    if call > 0 {
                        assert_eq!(action, kind.retry_action());
                    }
                    if call < 6 {
                        Err(WorkFailure {
                            kind,
                            message: format!("failure {call}"),
                        })
                    } else {
                        Ok(())
                    }
                }
            })
            .await;
        // Then
        let retryable = kind.retry_action() != RetryAction::Stop;
        assert_eq!(result.is_ok(), retryable);
        assert_eq!(calls.load(Ordering::SeqCst), if retryable { 7 } else { 1 });
        let records = queue.records("/repo").await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record.count, if retryable { 6 } else { 1 });
        assert_eq!(records[0].requires_attention, kind.requires_attention());
    }
}

#[tokio::test(start_paused = true)]
async fn test_作業列_期限で停止して他対象を妨げない() {
    // Given
    let queue = queue();
    let blocked = queue.execute(
        WorkKey::new("repository_scan", "slow"),
        RetryBackoff::ITEM,
        |_| std::future::pending::<Result<(), WorkFailure>>(),
    );
    let ready = queue.execute(
        WorkKey::new("repository_scan", "ready"),
        RetryBackoff::ITEM,
        |_| async { Ok(42) },
    );
    // When
    let (blocked, ready) = tokio::join!(blocked, ready);
    // Then
    assert_eq!(ready.unwrap(), 42);
    assert_eq!(blocked.unwrap_err().kind, FailureKind::Expired);
    assert_eq!(queue.records("slow").await[0].record.count, 1);
    assert!(queue.records("slow").await[0].requires_attention);
}

#[tokio::test(start_paused = true)]
async fn test_作業列_停止分類を周期から再投入しても繰り返さない() {
    // Given
    let queue = queue();
    let key = WorkKey::new("provider_session_title", "session");
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let job: Job = Arc::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(WorkFailure {
                kind: FailureKind::Corrupt,
                message: "corrupt".into(),
            })
        })
    });
    // When
    queue
        .enqueue(key.clone(), RetryBackoff::ITEM, job.clone())
        .await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    queue.enqueue(key, RetryBackoff::ITEM, job).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // Then
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_試行の失敗記録_下位の再試行と上位への伝播を二重計上しない() {
    let key = WorkKey::new("workflow_test", "retry-stage-count");
    let calls = AtomicUsize::new(0);
    let result = retry(shared(), key.clone(), RetryBackoff::ITEM, || {
        retry_stage(shared(), key.clone(), RetryBackoff::ITEM, || async {
            Err::<(), _>(WorkFailure {
                kind: if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    FailureKind::Temporary
                } else {
                    FailureKind::Internal
                },
                message: "failure".into(),
            })
        })
    })
    .await;
    assert_eq!(result.unwrap_err().kind, FailureKind::Internal);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let records = shared().records(&key.target).await;
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.record.count == 1));
}

#[tokio::test(start_paused = true)]
async fn test_監視の恒久失敗_全体の記録にも要対応を表示する() {
    let queue = queue();
    queue
        .observe(
            &WorkKey::new("review_comments_watch", "comments"),
            &WorkFailure {
                kind: FailureKind::Permission,
                message: "permission denied".into(),
            },
        )
        .await;
    let records = queue.records("*").await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.target, "comments");
    assert!(records[0].requires_attention);
}

#[tokio::test(start_paused = true)]
async fn test_workflow失敗_停止理由を保持し成功後は要対応を解消する() {
    let queue = queue();
    let key = WorkKey::new("workflow_recovery", "tree");
    queue
        .observe(
            &key,
            &WorkFailure {
                kind: FailureKind::StateRequired,
                message: "repair".into(),
            },
        )
        .await;
    assert!(queue.records("tree").await[0].requires_attention);
    queue
        .execute(key, RetryBackoff::RECOVERY, |_| async { Ok(()) })
        .await
        .unwrap();
    let records = queue.records("tree").await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record.count, 1);
    assert!(!records[0].requires_attention);
}

#[tokio::test(start_paused = true)]
async fn test_作業列_同一キーの実行重複を明示して先行の結果を保持する() {
    // Given
    let queue = queue();
    let key = WorkKey::new("repository_scan", "duplicate");
    let release = Arc::new(Notify::new());
    let entered = Arc::new(Notify::new());
    let first = {
        let queue = queue.clone();
        let key = key.clone();
        let release = release.clone();
        let entered = entered.clone();
        tokio::spawn(async move {
            queue
                .execute(key, RetryBackoff::ITEM, move |_| {
                    let release = release.clone();
                    let entered = entered.clone();
                    async move {
                        entered.notify_one();
                        release.notified().await;
                        Ok(42)
                    }
                })
                .await
        })
    };
    entered.notified().await;
    // When
    let second = queue
        .execute(key, RetryBackoff::ITEM, |_| async { Ok(7) })
        .await;
    release.notify_one();
    // Then
    assert_eq!(second.unwrap_err().kind, FailureKind::AlreadyPresent);
    assert_eq!(first.await.unwrap().unwrap(), 42);
}

#[tokio::test(start_paused = true)]
async fn test_対象の要対応_terminalとsessionは停止失敗を表示し成功で解除する() {
    // Given
    for operation in ["terminal_checkpoint", "provider_session_title"] {
        for kind in [
            FailureKind::StateRequired,
            FailureKind::Internal,
            FailureKind::Cancelled,
        ] {
            let queue = queue();
            let key = WorkKey::new(operation, "target");
            // When
            queue
                .execute(key.clone(), RetryBackoff::ITEM, move |_| async move {
                    Err::<(), _>(WorkFailure {
                        kind,
                        message: "failure".into(),
                    })
                })
                .await
                .unwrap_err();
            // Then
            assert_eq!(
                queue.records("target").await[0].requires_attention,
                kind.requires_attention()
            );
            queue
                .execute(key, RetryBackoff::ITEM, |_| async { Ok(()) })
                .await
                .unwrap();
            assert!(!queue.records("target").await[0].requires_attention);
            assert!(!queue.records_page("target", 0).await.requires_attention);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn test_失敗の記録_保持上限までページから欠落なく観測できる() {
    // Given
    let queue = queue();
    for index in 0..4096 {
        queue
            .observe(
                &WorkKey::new(&format!("operation-{index}"), "target"),
                &WorkFailure {
                    kind: FailureKind::StateRequired,
                    message: index.to_string(),
                },
            )
            .await;
    }
    // When
    let mut offset = 0;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = queue.records_page("target", offset).await;
        assert!(page.items.len() <= 100);
        assert!(page.requires_attention);
        for observation in page.items {
            let record = observation.record;
            assert!(seen.insert(record.operation));
            assert_eq!(record.target, "target");
            assert_eq!(record.kind, FailureKind::StateRequired);
            assert_eq!(record.count, 1);
            assert_eq!(record.first_observed_ms, record.last_observed_ms);
        }
        let Some(next) = page.next_offset else {
            break;
        };
        offset = next;
    }
    // Then
    assert_eq!(seen.len(), 4096);
}

#[tokio::test(start_paused = true)]
async fn test_作業列_多数の対象の再試行に共通の頻度上限が適用される() {
    // Given
    let queue = queue();
    let times = Arc::new(std::sync::Mutex::new(Vec::new()));
    let started = tokio::time::Instant::now();
    let mut tasks = Vec::new();
    for index in 0..110 {
        let queue = queue.clone();
        let times = times.clone();
        tasks.push(tokio::spawn(async move {
            let calls = AtomicUsize::new(0);
            queue
                .execute(
                    WorkKey::new("repository_scan", &index.to_string()),
                    RetryBackoff::ITEM,
                    move |_| {
                        let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                        let times = times.clone();
                        async move {
                            if first {
                                Err(WorkFailure {
                                    kind: FailureKind::Temporary,
                                    message: "busy".into(),
                                })
                            } else {
                                times.lock().unwrap().push(started.elapsed());
                                Ok(())
                            }
                        }
                    },
                )
                .await
                .unwrap();
        }));
    }
    // When
    for task in tasks {
        task.await.unwrap();
    }
    // Then
    let mut times = times.lock().unwrap().clone();
    times.sort();
    assert_eq!(times.len(), 110);
    assert!(times[99] < Duration::from_millis(100));
    for (index, elapsed) in times.iter().enumerate().skip(100) {
        assert!(*elapsed >= Duration::from_millis((index as u64 - 99) * 100));
    }
}

#[tokio::test(start_paused = true)]
async fn test_借用する再試行_同一キーを直列化し失敗回数を作業列に保持する() {
    // Given
    let queue = queue();
    let key = WorkKey::new("workflow_control_plane", "scoped");
    let calls = AtomicUsize::new(0);
    let durations = std::sync::Mutex::new(Vec::new());
    let start = tokio::time::Instant::now();
    // When
    let first = retry_with_scope(
        &queue,
        key.clone(),
        RetryBackoff::ITEM,
        || async {
            durations.lock().unwrap().push(start.elapsed());
            if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                Err(WorkFailure {
                    kind: FailureKind::RestartRequired,
                    message: "conflict".into(),
                })
            } else {
                Ok(42)
            }
        },
        true,
    );
    let second = retry_with_scope(
        &queue,
        key,
        RetryBackoff::ITEM,
        || async { Ok::<_, WorkFailure>(7) },
        true,
    );
    let (first, second) = tokio::join!(first, second);
    // Then
    assert_eq!(first.unwrap(), 42);
    assert_eq!(second.unwrap(), 7);
    assert_eq!(
        *durations.lock().unwrap(),
        [
            Duration::ZERO,
            Duration::from_millis(10),
            Duration::from_millis(60)
        ]
    );
    assert!(queue.state.lock().unwrap().jobs.is_empty());
}

#[tokio::test]
async fn test_要対応_取消後は対象表示と全体ページの両方から解除する() {
    // Given
    for operation in [
        "repository_scan",
        "terminal_checkpoint",
        "provider_session_title",
    ] {
        let queue = queue();
        let key = WorkKey::new(operation, "target");
        queue
            .observe(
                &key,
                &WorkFailure {
                    kind: FailureKind::StateRequired,
                    message: "repair".into(),
                },
            )
            .await;
        assert!(queue.records_page("target", 0).await.requires_attention);
        // When
        queue
            .observe(
                &key,
                &WorkFailure {
                    kind: FailureKind::Cancelled,
                    message: "cancel".into(),
                },
            )
            .await;
        // Then
        assert!(queue
            .records("target")
            .await
            .iter()
            .all(|record| !record.requires_attention));
        assert!(!queue.records_page("target", 0).await.requires_attention);
        assert!(!queue.records_page("*", 0).await.requires_attention);
    }
}

#[tokio::test]
async fn test_要対応の通知_設定と解除で同じ購読対象に通知する() {
    // Given
    use crate::domain::state_subscription::StateChangeSource;
    let queue = queue();
    let publisher = crate::usecase::state_subscription::StateSubscriptionPublisher::for_test();
    let mut changes = publisher.subscribe_changes();
    queue.set_publisher(publisher);
    let key = WorkKey::new("workflow_recovery", "tree");
    // When / Then
    queue
        .observe(
            &key,
            &WorkFailure {
                kind: FailureKind::StateRequired,
                message: "repair".into(),
            },
        )
        .await;
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::WorkspaceList
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
    queue.clear_attention(&key).await;
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::WorkspaceList
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
}

#[tokio::test(start_paused = true)]
async fn test_借用する再試行_取消で作業列の同じキーを解放する() {
    // Given
    let queue = queue();
    let key = WorkKey::new("workflow_control_plane", "cancelled");
    let entered = Notify::new();
    let mut work = Box::pin(retry_with_scope(
        &queue,
        key.clone(),
        RetryBackoff::ITEM,
        || async {
            entered.notify_one();
            std::future::pending::<Result<(), WorkFailure>>().await
        },
        true,
    ));
    // When
    tokio::select! {
        _ = &mut work => panic!("must remain pending"),
        _ = entered.notified() => {}
    }
    assert!(queue.state.lock().unwrap().jobs.contains_key(&key));
    drop(work);
    // Then
    assert!(!queue.state.lock().unwrap().jobs.contains_key(&key));
    assert_eq!(
        queue
            .execute(key, RetryBackoff::ITEM, |_| async { Ok(42) })
            .await
            .unwrap(),
        42
    );
}

#[test]
fn test_再試行_分類から待ち時間を選び失敗回数を保持する() {
    // Given
    let mut queue = WorkQueue::default();
    queue.add("a", Duration::ZERO);
    queue.get(Duration::ZERO);
    // When / Then
    let due = retry_due(
        &mut queue,
        &"a",
        FailureKind::RestartRequired,
        RetryBackoff::ITEM,
        Duration::ZERO,
        1.0,
    );
    assert_eq!(due, Duration::from_millis(10));
    queue.done(&"a", Some(due), false);
    assert_eq!(queue.get(Duration::from_millis(9)), None);
    assert_eq!(queue.get(due), Some("a"));
    let due = retry_due(
        &mut queue,
        &"a",
        FailureKind::Temporary,
        RetryBackoff::ITEM,
        due,
        1.0,
    );
    assert_eq!(due, Duration::from_millis(20));
    assert_eq!(queue.failure_count(&"a"), 2);
    queue.done(&"a", None, true);
    assert_eq!(queue.failure_count(&"a"), 0);
}

#[tokio::test(start_paused = true)]
async fn test_借用する再試行_restartを渡し最終失敗を返して仕事を解放する() {
    // Given
    let queue = queue();
    let actions = std::sync::Mutex::new(Vec::new());
    // When
    let result = run_borrowed(
        &queue,
        WorkKey::new("workflow_control_plane", "final-failure"),
        RetryBackoff::ITEM,
        |action| {
            let mut actions = actions.lock().unwrap();
            actions.push(action);
            let kind = if actions.len() == 1 {
                FailureKind::RestartRequired
            } else {
                FailureKind::InvalidInput
            };
            async move {
                Err::<(), _>(WorkFailure {
                    kind,
                    message: "failed".into(),
                })
            }
        },
        true,
        true,
    )
    .await;
    // Then
    assert_eq!(result.unwrap_err().kind, FailureKind::InvalidInput);
    assert_eq!(
        *actions.lock().unwrap(),
        [RetryAction::Retry, RetryAction::Restart]
    );
    assert!(queue.state.lock().unwrap().jobs.is_empty());
}
