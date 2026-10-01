use super::*;

#[tokio::test]
async fn test_terminal入力識別子_上限を受け付け超過を拒否する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None);
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
        .start_terminal("client", &target, Some(&at_limit), None)
        .await;
    let rejected = usecase
        .start_terminal("client", &target, Some(&over_limit), None)
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
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None);
    let target = SubscriptionTarget::RepositoryPaths;
    // When / Then
    for input in [String::new(), " \t\n".into(), "あ".repeat(43)] {
        let error = usecase
            .start_terminal("client", &target, Some(&input), None)
            .await
            .unwrap_err();
        assert!(matches!(
            error.source,
            StateReadFailure::InvalidTerminalInput
        ));
        assert_eq!(error.message, "Invalid terminal input identity");
    }
    let error = usecase
        .start_terminal("client", &target, Some(&"あ".repeat(42)), None)
        .await
        .unwrap_err();
    assert_eq!(error.message, "Not a terminal target");
}

#[test]
fn test_terminalのclient管理_二重openを拒否し閉じた購読の処理報告を拒否する() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None);
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
    subscribed: Mutex<HashSet<(String, SubscriptionTarget)>>,
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
        _: Option<(&str, u64)>,
    ) -> Result<Option<usize>, StateReadError> {
        *self.starts.lock() += 1;
        if let Some(error) = *self.start_error.lock() {
            return Err(StateReadError::from_error(error));
        }
        self.subscribed
            .lock()
            .insert((client.into(), target.clone()));
        Ok(*self.pending.lock())
    }
    fn stop(&self, client: &str, target: &SubscriptionTarget) -> Result<(), SubscriptionError> {
        *self.stops.lock() += 1;
        self.subscribed
            .lock()
            .remove(&(client.into(), target.clone()));
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
async fn test_terminal入力識別子_省略時はclientの識別子で開始する() {
    // Given
    use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
    let (terminal, gateway, _, surface) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(output, Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    // When
    usecase
        .start_terminal("client", &target, None, None)
        .await
        .unwrap();
    // Then
    assert_eq!(
        usecase.test_input_id("client", &target).as_deref(),
        Some("client")
    );
    gateway
        .write_attached(&surface.session_key, "client", 1, "input")
        .unwrap();
}

#[tokio::test]
async fn test_terminal入力識別子_補ったclientが上限超過なら入力不正を返す() {
    // Given
    let usecase = TerminalSubscriptionUsecase::new(Arc::new(FakeOutput::default()), None);
    let client = "a".repeat(129);
    // When
    let error = usecase
        .start_terminal(&client, &SubscriptionTarget::RepositoryPaths, None, None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    let stopped = usecase.clone();
    let stopping_target = target.clone();
    *gateway.before_output_order.lock() = Some(Box::new(move || {
        stopped
            .stop_subscription("client", &stopping_target)
            .unwrap()
    }));
    // When
    let result = usecase
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
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
            .stop_subscription("client", &stopping_target)
            .unwrap();
    }));
    // When
    let result = usecase
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
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
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
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
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    let removed = gateway.clone();
    *gateway.during_output_order.lock() = Some(Box::new(move || {
        removed.remove_surface(1).unwrap();
    }));
    // When
    assert!(usecase
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    // When
    let error = usecase
        .start_terminal("client", &target, Some("input"), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_terminal("client", &target, Some("input"), None)
        .await
        .unwrap();
    // When
    usecase.terminal_processed("client", &target, 5000).unwrap();
    // Then
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        Some(1000)
    );
    usecase.stop_subscription("client", &target).unwrap();
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    for client in ["first", "second"] {
        usecase.open_client(client.into()).unwrap();
        usecase
            .start_terminal(client, &target, Some(client), None)
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
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal));
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
    let output = Arc::new(FakeOutput::default());
    let usecase = TerminalSubscriptionUsecase::new(output, None);
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    usecase.open_client("client".into()).unwrap();
    usecase
        .clients
        .lock()
        .get_mut("client")
        .unwrap()
        .insert(target.clone(), "input".into());
    usecase
        .terminal_resets
        .lock()
        .insert(target.clone(), HashSet::from(["client".into()]));
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    usecase.workers.lock().insert(target.clone(), task);
    // When
    usecase.stop_subscription("client", &target).unwrap();
    // Then
    assert_ended(&usecase, "client", &target);
    assert!(!usecase.terminal_resets.lock().contains_key(&target));
    assert_eq!(usecase.test_worker_count(), 0);
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
    // Given / When: a failed worker has already removed itself.
    usecase
        .terminal_resets
        .lock()
        .insert(target.clone(), HashSet::from(["client".into()]));
    usecase.stop_subscription("client", &target).unwrap();
    // Then
    assert!(!usecase.terminal_resets.lock().contains_key(&target));
}

#[tokio::test]
async fn test_terminal切断_閉じたclientをreset記録から消し残る購読者を保持する() {
    // Given
    let (terminal, gateway, hub, surface) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let output = Arc::new(FakeOutput::default());
    *output.pending.lock() = Some(6000);
    let usecase = TerminalSubscriptionUsecase::new(output, Some(terminal));
    let target = SubscriptionTarget::Terminal(surface.owner);
    for client in ["closed", "active"] {
        usecase.open_client(client.into()).unwrap();
        usecase
            .start_terminal(client, &target, Some(client), None)
            .await
            .unwrap();
    }
    usecase.terminal_resets.lock().insert(
        target.clone(),
        HashSet::from(["closed".into(), "active".into()]),
    );
    let task = tokio::spawn(std::future::pending::<()>());
    let abort = task.abort_handle();
    usecase.workers.lock().insert(target.clone(), task);
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
    assert!(!abort.is_finished());
    // When
    usecase.close_client("active");
    // Then
    assert_ended(&usecase, "active", &target);
    assert!(!hub.test_subscribed(&surface.session_key, "active"));
    assert!(usecase.terminal_resets.lock().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
    tokio::task::yield_now().await;
    assert!(abort.is_finished());
}
