use super::*;
use crate::domain::client_operation::policy;

fn identity(command: &str) -> OperationIdentity {
    OperationIdentity {
        command: command.into(),
        fingerprint: [1; 32],
        target: None,
    }
}

fn completion(result: i32, started_watch: Option<u64>) -> OperationCompletion<i32> {
    OperationCompletion {
        result,
        started_watch,
        stopped_watch: None,
    }
}

#[tokio::test]
async fn test_操作手順_別の入口も同じ受理と実行と完了記録を通す() {
    // Given
    let operations = ClientOperationUsecase::new(
        "current".into(),
        Arc::new(|| 0),
        Arc::new(|_| Box::pin(async {})),
    );
    let other_entry = operations.clone();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let run = || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(completion(42, None))
    };
    // When
    let first = operations.execute(
        "operation".into(),
        "current",
        identity("append_review_comment"),
        &[],
        &RecoveryAttempt::default(),
        run,
    );
    let duplicate = other_entry.execute(
        "operation".into(),
        "current",
        identity("append_review_comment"),
        &[],
        &RecoveryAttempt::default(),
        run,
    );
    // Then
    assert_eq!(duplicate.await, OperationDecision::Pending);
    assert_eq!(first.await, OperationDecision::Completed(42));
    assert_eq!(
        other_entry.query("operation", "current"),
        OperationDecision::Completed(42)
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    other_entry.acknowledge("operation", false).await;
    assert_eq!(
        operations.query("operation", "current"),
        OperationDecision::Unknown
    );
}

#[tokio::test]
async fn test_監視手順_受領前切断と期限破棄の停止を共有する() {
    // Given
    let now = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let released = Arc::new(Mutex::new(Vec::new()));
    let operations = ClientOperationUsecase::new(
        "current".into(),
        {
            let now = now.clone();
            Arc::new(move || now.load(std::sync::atomic::Ordering::SeqCst))
        },
        {
            let released = released.clone();
            Arc::new(move |id| {
                let released = released.clone();
                Box::pin(async move {
                    released.lock().unwrap().push(id);
                })
            })
        },
    );
    // When / Then
    let execute = |id: &str, watcher| {
        operations.execute(
            id.into(),
            "current",
            identity("start_watching"),
            &[],
            &RecoveryAttempt {
                connection: "socket",
                ..Default::default()
            },
            move || std::future::ready(completion(watcher, Some(watcher as u64))),
        )
    };
    let running = execute("running", 1);
    operations.disconnect("socket").await;
    running.await;
    assert_eq!(*released.lock().unwrap(), vec![1]);
    execute("confirmed", 2).await;
    operations.acknowledge("confirmed", false).await;
    operations.disconnect("socket").await;
    assert_eq!(
        operations.query("confirmed", "current"),
        OperationDecision::Completed(2)
    );
    execute("expired", 3).await;
    now.store(
        policy::OPERATION_RETENTION_MS,
        std::sync::atomic::Ordering::SeqCst,
    );
    operations.maintain().await;
    assert_eq!(
        operations.query("expired", "current"),
        OperationDecision::Unknown
    );
    assert_eq!(*released.lock().unwrap(), vec![1, 3]);
}

#[tokio::test]
async fn test_期限後の再試行_破棄済みcallerの元のキーで重複を防ぐ() {
    // Given
    let now = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let journal = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let operations = ClientOperationUsecase::new(
        "current".into(),
        {
            let now = now.clone();
            Arc::new(move || now.load(std::sync::atomic::Ordering::SeqCst))
        },
        Arc::new(|_| Box::pin(async {})),
    );
    let run = || {
        let journal = journal.clone();
        let effects = effects.clone();
        async move {
            let result = *journal
                .lock()
                .unwrap()
                .entry("original-caller")
                .or_insert_with(|| {
                    effects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    42
                });
            completion(result, None)
        }
    };
    operations
        .execute(
            "op".into(),
            "current",
            identity("create_agent_session"),
            &[],
            &RecoveryAttempt::default(),
            run,
        )
        .await;
    now.store(
        policy::OPERATION_RETENTION_MS,
        std::sync::atomic::Ordering::SeqCst,
    );
    // When / Then
    let expired = RecoveryAttempt {
        sent: true,
        expired: true,
        ..Default::default()
    };
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &expired,
                run
            )
            .await,
        OperationDecision::Unknown
    );
    let retry = RecoveryAttempt {
        sent: true,
        user_retry: true,
        ..Default::default()
    };
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &retry,
                run
            )
            .await,
        OperationDecision::Completed(42)
    );
    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_保持上限の再試行_破棄済みcallerを再受理しても副作用を重複させない() {
    // Given
    let operations = ClientOperationUsecase::new(
        "current".into(),
        Arc::new(|| 0),
        Arc::new(|_| Box::pin(async {})),
    );
    let journal = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let run = || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let journal = journal.clone();
        let effects = effects.clone();
        async move {
            let result = *journal
                .lock()
                .unwrap()
                .entry("original-caller")
                .or_insert_with(|| {
                    effects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    42
                });
            completion(result, None)
        }
    };
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &RecoveryAttempt::default(),
                run
            )
            .await,
        OperationDecision::Completed(42)
    );
    for index in 0..policy::MAX_UNACKNOWLEDGED_OPERATIONS {
        operations
            .execute(
                index.to_string(),
                "current",
                identity("write_terminal_surface"),
                &[],
                &RecoveryAttempt::default(),
                || std::future::ready(completion(0, None)),
            )
            .await;
    }
    operations.acknowledge("0", false).await;
    // When / Then
    assert_eq!(
        operations.query("op", "current"),
        OperationDecision::Unknown
    );
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &RecoveryAttempt {
                    sent: true,
                    ..Default::default()
                },
                run
            )
            .await,
        OperationDecision::Unknown
    );
    let retry = RecoveryAttempt {
        sent: true,
        user_retry: true,
        ..Default::default()
    };
    let retried = operations.execute(
        "op".into(),
        "current",
        identity("create_agent_session"),
        &[],
        &retry,
        run,
    );
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &retry,
                run
            )
            .await,
        OperationDecision::Pending
    );
    assert_eq!(retried.await, OperationDecision::Completed(42));
    assert_eq!(
        operations
            .execute(
                "op".into(),
                "current",
                identity("create_agent_session"),
                &[],
                &retry,
                run
            )
            .await,
        OperationDecision::Completed(42)
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_監視の受領確認_保持期限後の遅延成功を有効として確定しない() {
    // Given
    let now = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stopped = Arc::new(Mutex::new(Vec::new()));
    let operations = ClientOperationUsecase::new(
        "current".into(),
        {
            let now = now.clone();
            Arc::new(move || now.load(std::sync::atomic::Ordering::SeqCst))
        },
        {
            let stopped = stopped.clone();
            Arc::new(move |id| {
                let stopped = stopped.clone();
                Box::pin(async move {
                    stopped.lock().unwrap().push(id);
                })
            })
        },
    );
    operations
        .execute(
            "watch".into(),
            "current",
            identity("start_watching"),
            &[],
            &RecoveryAttempt {
                connection: "socket",
                ..Default::default()
            },
            || std::future::ready(completion(42, Some(42))),
        )
        .await;
    // When
    now.store(
        policy::OPERATION_RETENTION_MS,
        std::sync::atomic::Ordering::SeqCst,
    );
    let active = operations.acknowledge("watch", false).await;
    operations.maintain().await;
    // Then
    assert!(!active);
    assert_eq!(*stopped.lock().unwrap(), vec![42]);
    assert_eq!(
        operations.query("watch", "current"),
        OperationDecision::Unknown
    );
}
