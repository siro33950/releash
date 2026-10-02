use super::*;

#[tokio::test]
async fn test_terminal入力識別子_上限を受け付け超過を拒否する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(
        Arc::new(FakeOutput::default()),
        None,
        crate::test_support::state_subscription::terminal_driver(),
    );

    let target = SubscriptionTarget::Terminal(
        crate::domain::terminal_surface::TerminalSurfaceOwner::workspace(
            crate::domain::workspace_tree::WorkspaceIdentity::new("/repo"),
        )
        .unwrap(),
    );
    let at_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES);
    let over_limit = "a".repeat(TERMINAL_INPUT_ID_MAX_BYTES + 1);
    // When
    let accepted = usecase
        .start_terminal("client", &target, &at_limit, None)
        .await;
    let rejected = usecase
        .start_terminal("client", &target, &over_limit, None)
        .await;
    // Then
    assert!(matches!(
        accepted,
        Err(StateReadError {
            source: StateReadFailure::Technical(_),
            ..
        })
    ));
    assert!(matches!(
        rejected,
        Err(StateReadError {
            source: StateReadFailure::InvalidTerminalInput,
            ..
        })
    ));
}

#[tokio::test]
async fn test_terminal入力識別子_空白とバイト上限を変えず検査する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(
        Arc::new(FakeOutput::default()),
        None,
        crate::test_support::state_subscription::terminal_driver(),
    );

    let target = SubscriptionTarget::RepositoryPaths;
    // When / Then
    for input in [String::new(), " \t\n".into(), "あ".repeat(43)] {
        let error = usecase
            .start_terminal("client", &target, &input, None)
            .await
            .unwrap_err();
        assert!(matches!(
            error.source,
            StateReadFailure::InvalidTerminalInput
        ));
        assert_eq!(error.message, "Invalid terminal input identity");
    }
    let error = usecase
        .start_terminal("client", &target, &"あ".repeat(42), None)
        .await
        .unwrap_err();
    assert_eq!(error.message, "Not a terminal target");
}

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
    let error = usecase
        .terminal_processed("client", &SubscriptionTarget::RepositoryPaths, 5000)
        .unwrap_err();
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

#[derive(Default)]
struct FakeOutput {
    subscribed: Mutex<HashSet<(String, SubscriptionTarget, String)>>,
    pending: Mutex<Option<usize>>,
    start_error: Mutex<Option<SubscriptionError>>,
    starts: Mutex<usize>,
    stops: Mutex<usize>,
    snapshots: Mutex<Vec<(u64, u64, StateValue)>>,
    failures: Mutex<Vec<StateReadError>>,
}

impl TerminalSubscriptionOutput for FakeOutput {
    fn start(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
        _: Option<(&str, u64)>,
    ) -> Result<Option<usize>, StateReadError> {
        *self.starts.lock() += 1;
        if let Some(error) = *self.start_error.lock() {
            return Err(StateReadError::from_error(error));
        }
        self.subscribed
            .lock()
            .insert((client.into(), target.clone(), input_id.into()));
        Ok(*self.pending.lock())
    }
    fn stop(
        &self,
        client: &str,
        target: &SubscriptionTarget,
        input_id: &str,
    ) -> Result<(), SubscriptionError> {
        *self.stops.lock() += 1;
        self.subscribed
            .lock()
            .remove(&(client.into(), target.clone(), input_id.into()));
        Ok(())
    }
    fn set_snapshot(
        &self,
        _: &SubscriptionTarget,
        generation: u64,
        sequence: u64,
        snapshot: StateValue,
    ) -> Result<(), SubscriptionError> {
        self.snapshots.lock().push((generation, sequence, snapshot));
        Ok(())
    }
    fn publish_failure(
        &self,
        _: &SubscriptionTarget,
        error: StateReadError,
    ) -> Result<(), SubscriptionError> {
        self.failures.lock().push(error);
        Ok(())
    }
}

fn assert_ended(usecase: &TerminalSubscriptionUsecase, client: &str, target: &SubscriptionTarget) {
    assert!(usecase.test_input_id(client, target).is_none());
    assert!(matches!(
        usecase
            .terminal_processed(client, target, 5000)
            .unwrap_err()
            .source,
        StateReadFailure::TerminalSubscriptionEnded
    ));
}

#[tokio::test]
async fn test_terminal入力識別子_明示した購読識別子で開始する() {
    // Given
    use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
    let (terminal, gateway, _, surface) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(
        output,
        Some(terminal),
        crate::test_support::state_subscription::terminal_driver(),
    );

    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    // When
    usecase
        .start_terminal("client", &target, "input", None)
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
async fn test_terminal入力識別子_clientと異なる購読識別子も検査する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(
        Arc::new(FakeOutput::default()),
        None,
        crate::test_support::state_subscription::terminal_driver(),
    );

    let client = "a".repeat(129);
    // When
    let error = usecase
        .start_terminal(
            "client",
            &SubscriptionTarget::RepositoryPaths,
            &client,
            None,
        )
        .await
        .unwrap_err();
    // Then
    assert!(matches!(
        error.source,
        StateReadFailure::InvalidTerminalInput
    ));
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
    *gateway.before_output_order.lock() = Some(Box::new(move || {
        stopped
            .stop_subscription("client", &stopping_target, "input")
            .unwrap()
    }));
    // When
    let result = usecase
        .start_terminal("client", &target, "input", None)
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
            .stop_subscription("client", &stopping_target, "input")
            .unwrap();
    }));
    // When
    let result = usecase
        .start_terminal("client", &target, "input", None)
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
        .start_terminal("client", &target, "input", None)
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
        .start_terminal("client", &target, "input", None)
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
        .start_terminal("client", &target, "input", None)
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
        .start_terminal("client", &target, "input", None)
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
        .start_terminal("client", &target, "input", None)
        .await
        .unwrap();
    // When
    usecase.terminal_processed("client", &target, 5000).unwrap();
    // Then
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        Some(1000)
    );
    usecase
        .stop_subscription("client", &target, "input")
        .unwrap();
    assert_ended(&usecase, "client", &target);
    assert!(!hub.test_subscribed(&surface.session_key, "client"));
    assert!(output.subscribed.lock().is_empty());
}

async fn wait_workers(usecase: &TerminalSubscriptionUsecase) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while usecase.test_worker_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_terminal作り直し予約_一つのworkerで追加clientのresetも併合する() {
    // Given
    use super::super::io_usecase::io_usecase_tests::FakePtyGateway;
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
            .start_terminal(client, &target, client, None)
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
    assert!(usecase.terminal_resets.lock().is_empty());
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
    use super::super::io_usecase::io_usecase_tests::FakePtyGateway;
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
async fn test_terminal停止_購読者が全員止まれば対象のreset記録とworkerを消す() {
    // Given
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(output, None, requests);

    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    usecase.open_client("client".into()).unwrap();
    usecase
        .clients
        .lock()
        .get_mut("client")
        .unwrap()
        .insert(target.clone(), HashSet::from(["input".into()]));
    usecase
        .terminal_resets
        .lock()
        .insert(target.clone(), HashSet::from(["client".into()]));
    usecase.schedule_terminal_refresh(vec![], target.clone());
    let mut request = receiver.recv().await.unwrap();
    // When
    usecase
        .stop_subscription("client", &target, "input")
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
        .stop_subscription("client", &target, "input")
        .unwrap();
    // Then
    assert!(!usecase.terminal_resets.lock().contains_key(&target));
}

#[tokio::test]
async fn test_terminal切断_閉じたclientをreset記録から消し残る購読者を保持する() {
    // Given
    let (terminal, gateway, hub, surface) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let output = Arc::new(FakeOutput::default());
    *output.pending.lock() = Some(6000);
    let usecase = TerminalSubscriptionUsecase::new(output, Some(terminal), requests);

    let target = SubscriptionTarget::Terminal(surface.owner);
    for client in ["closed", "active"] {
        usecase.open_client(client.into()).unwrap();
        usecase
            .start_terminal(client, &target, client, None)
            .await
            .unwrap();
    }
    usecase.terminal_resets.lock().insert(
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
        usecase.terminal_resets.lock()[&target],
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
    assert!(usecase.terminal_resets.lock().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
    assert!(matches!(
        request.cancelled.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
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

pub(crate) fn add_reset(
    usecase: &TerminalSubscriptionUsecase,
    target: &SubscriptionTarget,
    client: &str,
) {
    usecase
        .terminal_resets
        .lock()
        .entry(target.clone())
        .or_default()
        .insert(client.into());
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
        .start_terminal("client", &target, "x", None)
        .await
        .unwrap();
    usecase
        .start_terminal("client", &target, "y", None)
        .await
        .unwrap();
    assert!(gateway
        .write_attached(&surface.session_key, "x", 0, "old")
        .is_err());
    // When
    usecase.stop_subscription("client", &target, "x").unwrap();
    // Then
    assert!(hub.test_subscribed(&surface.session_key, "client"));
    assert_eq!(output.subscribed.lock().len(), 1);
    gateway
        .write_attached(&surface.session_key, "y", 1, "new")
        .unwrap();
    usecase.terminal_processed("client", &target, 5000).unwrap();
    usecase.stop_subscription("client", &target, "y").unwrap();
    assert!(!hub.test_subscribed(&surface.session_key, "client"));
    assert!(output.subscribed.lock().is_empty());
    assert!(gateway
        .write_attached(&surface.session_key, "y", 2, "stopped")
        .is_err());
}

pub(super) struct TestTerminalDelivery<'a> {
    pub(super) publisher: &'a dyn TerminalSubscriptionOutput,
    pub(super) client: &'a str,
    pub(super) target: &'a SubscriptionTarget,
    pub(super) input: &'a str,
    pub(super) cursor: Option<(&'a str, u64)>,
}

impl StateSubscriptionDelivery for TestTerminalDelivery<'_> {
    fn start(&self) -> Result<Option<usize>, StateReadError> {
        self.publisher
            .start(self.client, self.target, self.input, self.cursor)
    }
    fn claim(&self) -> bool {
        true
    }
    fn finish(&self, _: &HashSet<SubscriptionTarget>) -> Result<(), SubscriptionError> {
        self.publisher.stop(self.client, self.target, self.input)
    }
}

#[tokio::test]
async fn test_terminal購読再利用_同じclientとidの新しい入力先へ古い停止が作用しない() {
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
    subscriptions
        .terminal
        .stop_delivery("client", &target, "x", &old)
        .unwrap();
    old.finish(&Default::default()).unwrap();
    assert!(subscriptions
        .usecase
        .test_presenter()
        .unwrap()
        .lookup("x")
        .is_some());
    assert!(hub.test_subscribed(&surface.session_key, "client"));
    gateway
        .write_attached(&surface.session_key, "x", 1, "new")
        .unwrap();
}
