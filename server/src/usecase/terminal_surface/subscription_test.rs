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

#[tokio::test]
async fn test_terminal作り直し予約_一つのworkerで追加clientのresetも併合する() {
    // Given
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None, requests);
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    usecase.schedule_terminal_refresh(vec!["first".into()], target.clone());
    let request = receiver.recv().await.unwrap();
    // When
    usecase.schedule_terminal_refresh(vec!["second".into()], target.clone());
    // Then
    assert_eq!(request.target, target);
    assert!(receiver.try_recv().is_err());
    assert_eq!(usecase.test_worker_count(), 1);
    assert!(!usecase.workers.lock()[&target].is_closed());
    assert_eq!(
        usecase.terminal_resets.lock()[&target],
        HashSet::from(["first".into(), "second".into()])
    );
}

fn resizable_terminal(
    gateway: Arc<crate::usecase::terminal_surface::test_helpers::FakePtyGateway>,
) -> Arc<super::super::application::TerminalSurfaceApplication> {
    let hub =
        Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
    Arc::new(super::super::application::TerminalSurfaceApplication::new(
        Arc::new(crate::usecase::test_helpers::NoopPerformance),
        gateway,
        Arc::new(
            crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(
                hub.event_sender(),
            ),
        ),
        hub,
        crate::test_support::terminal_config(""),
    ))
}

#[tokio::test]
async fn test_terminalサイズ更新_resize後に購読者へsnapshotの再送を求める() {
    use crate::usecase::terminal_surface::test_helpers::{workspace_owner, FakePtyGateway};
    // Given
    let owner = workspace_owner("/repo");
    let gateway = Arc::new(FakePtyGateway::new());
    let output = Arc::new(FakeOutput::default());
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let usecase = TerminalSubscriptionUsecase::new(
        output.clone(),
        Some(resizable_terminal(gateway.clone())),
        requests,
    );
    // When
    usecase.prepare_resize(owner.clone(), 30, 100).unwrap()().unwrap();
    // Then
    assert_eq!(
        gateway.resizes.lock().as_slice(),
        [(owner.stable_key(), 30, 100)]
    );
    assert_eq!(
        output.required.lock().as_slice(),
        [SubscriptionTarget::Terminal(owner)]
    );
    assert!(receiver.try_recv().is_err());
}

async fn snapshot_shell_state(
    state: Option<crate::domain::terminal_surface::TerminalShellState>,
    exited: bool,
) -> super::super::application::TerminalSurfaceSnapshotDto {
    use crate::usecase::terminal_surface::{
        application::TerminalSurfaceApplication,
        test_helpers::{workspace_owner, FakePtyGateway},
    };
    // Given
    let owner = workspace_owner("/repo");
    let mut gateway = FakePtyGateway::new();
    gateway.surface = Some(
        crate::domain::terminal_surface::entities::TerminalSurface::new(1, owner.clone(), None),
    );
    if exited {
        gateway
            .surface
            .as_mut()
            .unwrap()
            .mark_exited(1.into(), Some(0))
            .unwrap();
    }
    gateway.shell_state = state;
    let hub =
        Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
    let terminal = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(crate::usecase::test_helpers::NoopPerformance), Arc::new(gateway),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub, crate::test_support::terminal_config(""),
    ));
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(
        output.clone(),
        Some(terminal),
        crate::test_support::state_subscription::terminal_driver(),
    );
    // When
    usecase
        .refresh_terminal(&SubscriptionTarget::Terminal(owner))
        .await
        .unwrap();
    let values = output.snapshots.lock();
    let StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(snapshot)) = &values[0].2 else {
        panic!("snapshot expected")
    };
    snapshot.clone()
}

#[tokio::test]
async fn test_terminal_snapshot_gatewayのshell状態を付ける() {
    let state = crate::domain::terminal_surface::TerminalShellState {
        cwd: Some("/next".into()),
        has_foreground_process: true,
    };
    // When / Then
    assert_eq!(
        snapshot_shell_state(Some(state.clone()), false)
            .await
            .shell_state,
        Some(state)
    );
}

#[tokio::test]
async fn test_terminal_snapshot_gatewayがnoneならshell状態を付けない() {
    // When / Then
    assert_eq!(snapshot_shell_state(None, false).await.shell_state, None);
}

#[tokio::test]
async fn test_terminal_snapshot_終了後の前面処理無しの状態を配信する() {
    // Given
    let mut status = crate::domain::terminal_surface::entities::TerminalShellStatus::default();
    status.observe(None, false, "/last".into(), true);
    status.mark_exited();
    // When
    let snapshot = snapshot_shell_state(status.latest(), true).await;
    // Then
    assert!(snapshot.is_exited);
    assert_eq!(snapshot.exit_code, Some(0));
    assert_eq!(
        snapshot.shell_state,
        Some(crate::domain::terminal_surface::TerminalShellState {
            cwd: Some("/last".into()),
            has_foreground_process: false,
        })
    );
}
