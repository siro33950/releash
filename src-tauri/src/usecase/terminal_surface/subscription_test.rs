use super::*;
use crate::usecase::terminal_surface::test_helpers::subscription::*;
#[test]
fn test_terminalのclient管理_二重openを拒否し閉じた購読の処理報告を拒否する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(
        Arc::new(FakeOutput::default()),
        None,
        crate::test_support::state_subscription::terminal_driver(),
    );

    usecase.open_client("client".into()).unwrap();
    // When / Then
    assert_eq!(
        usecase.open_client("client".into()),
        Err(SubscriptionError::AlreadyExists)
    );
    usecase.close_client("client");
    let error = usecase.terminal_processed("client", 5000).unwrap_err();
    assert!(matches!(
        error.source,
        StateReadFailure::TerminalSubscriptionEnded
    ));
    assert_eq!(error.message, "Terminal subscription ended");
    usecase.open_client("client".into()).unwrap();
    usecase.schedule_terminal_refresh(vec!["client".into()], SubscriptionTarget::RepositoryPaths);
    assert!(usecase.terminal_resets.lock().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
}

#[tokio::test]
async fn test_terminal停止_購読者が全員止まれば対象のreset記録とworkerを消す() {
    // Given
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), None, requests);

    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    usecase.open_client("client".into()).unwrap();
    usecase
        .clients
        .lock()
        .get_mut("client")
        .unwrap()
        .insert(target.clone(), vec!["input".into()]);
    usecase
        .terminal_resets
        .lock()
        .insert(target.clone(), HashSet::from(["client".into()]));
    usecase.schedule_terminal_refresh(vec![], target.clone());
    let mut request = receiver.recv().await.unwrap();
    // When
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
    // Then
    assert_ended(&usecase, "client", &target);
    assert!(!usecase.terminal_resets.lock().contains_key(&target));
    assert_eq!(usecase.test_worker_count(), 0);
    assert!(matches!(
        request.cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    // Given / When: a failed worker has already removed itself.
    usecase
        .terminal_resets
        .lock()
        .insert(target.clone(), HashSet::from(["client".into()]));
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
    // Then
    assert!(!usecase.terminal_resets.lock().contains_key(&target));
}

#[test]
fn test_terminal作り直し予約_駆動部が終了したら失敗を配信する() {
    // Given
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(
        output.clone(),
        None,
        tokio::sync::mpsc::unbounded_channel().0,
    );
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    // When
    usecase.schedule_terminal_refresh(vec!["client".into()], target);
    // Then
    assert_eq!(usecase.test_worker_count(), 0);
    let failures = output.failures.lock();
    assert_eq!(failures.len(), 1);
    assert!(
        matches!(&failures[0].source, StateReadFailure::Subscription(error)
        if **error == SubscriptionError::StreamEnded)
    );
}

#[tokio::test]
async fn test_terminal作り直し予約_異常終了したworkerを再登録する() {
    // Given
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None, requests);
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    usecase.schedule_terminal_refresh(vec!["client".into()], target.clone());
    drop(receiver.recv().await.unwrap());
    // When
    usecase.schedule_terminal_refresh(vec!["client".into()], target.clone());
    // Then
    let request = receiver.try_recv().unwrap();
    assert_eq!(request.target, target);
    assert_eq!(usecase.test_worker_count(), 1);
    assert!(!usecase.workers.lock()[&target].is_closed());
}
