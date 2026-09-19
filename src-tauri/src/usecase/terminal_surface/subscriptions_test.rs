use super::super::io_usecase::io_usecase_tests::FakePtyGateway;
use super::*;
use crate::domain::terminal_surface::{
    entities::TerminalSurface, gateway::*, TerminalProcessState, TerminalSurfaceCheckpoint,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

#[derive(Default)]
struct Events(parking_lot::Mutex<Vec<mpsc::UnboundedSender<TerminalSurfaceEvent>>>);
struct Cancellation(parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>);
impl TerminalSurfaceEventCancellation for Cancellation {
    fn cancel(&self) {
        if let Some(sender) = self.0.lock().take() {
            let _ = sender.send(());
        }
    }
}
struct SubscriptionEvents {
    receiver: mpsc::UnboundedReceiver<TerminalSurfaceEvent>,
    canceled: tokio::sync::oneshot::Receiver<()>,
}
impl TerminalSurfaceEventSubscription for SubscriptionEvents {
    fn recv(
        &mut self,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<TerminalSurfaceEvent, TerminalSurfaceEventReceiveError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            tokio::select! {
                biased;
                _ = &mut self.canceled => Err(TerminalSurfaceEventReceiveError::Closed),
                event = self.receiver.recv() => event.ok_or(TerminalSurfaceEventReceiveError::Closed),
            }
        })
    }
}
impl TerminalSurfaceEventSource for Events {
    fn subscribe(&self) -> TerminalSurfaceEventStream {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancel, canceled) = tokio::sync::oneshot::channel();
        self.0.lock().push(sender);
        TerminalSurfaceEventStream {
            subscription: Box::new(SubscriptionEvents { receiver, canceled }),
            cancellation: Arc::new(Cancellation(parking_lot::Mutex::new(Some(cancel)))),
        }
    }
}
fn owner() -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap()
}
fn fixture(
    exited: bool,
) -> (
    TerminalSubscriptionUsecase,
    Arc<FakePtyGateway>,
    Arc<Events>,
) {
    let mut gateway = FakePtyGateway::new();
    let mut surface = TerminalSurface::with_checkpoint(
        1,
        owner(),
        None,
        TerminalSurfaceCheckpoint::empty(80, 24),
    );
    if exited {
        surface.process_state = TerminalProcessState::Exited { exit_code: Some(0) };
    }
    gateway.surface = Some(surface);
    let gateway = Arc::new(gateway);
    let events = Arc::new(Events::default());
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        events.clone(),
    ));
    (
        TerminalSubscriptionUsecase::new(application),
        gateway,
        events,
    )
}

#[tokio::test]
async fn test_terminal購読_旧attachmentの終了と解放は再attachした出力と入力を失効させない() {
    // Given
    let (usecase, gateway, events) = fixture(false);
    let mut old = usecase.subscribe("old".into()).unwrap();
    old.next().await.unwrap();
    usecase
        .attach("old", "same".into(), "first".into(), &owner())
        .unwrap();
    old.next().await.unwrap();
    let mut current = usecase.subscribe("current".into()).unwrap();
    current.next().await.unwrap();
    // When
    usecase
        .attach("current", "same".into(), "second".into(), &owner())
        .unwrap();
    current.next().await.unwrap();
    assert!(
        matches!(old.next().await, Some(TerminalSubscriptionEvent::Closed { stream_id, .. }) if stream_id == "first")
    );
    drop(old);
    // Then
    assert!(gateway.deactivated.lock().is_empty());
    events
        .0
        .lock()
        .last()
        .unwrap()
        .send(TerminalSurfaceEvent::Output {
            session_key: owner().stable_key(),
            data: "live".into(),
            sequence: 1,
        })
        .unwrap();
    assert!(
        matches!(current.next().await, Some(TerminalSubscriptionEvent::Item { stream_id, item: TerminalSurfaceStreamItem::Output { data, .. }, .. }) if stream_id == "second" && data.as_ref() == "live")
    );
    usecase
        .application
        .write_attached(&owner(), "same", 0, None, "input")
        .unwrap();
    drop(current);
    assert_eq!(*gateway.deactivated.lock(), ["same"]);
    let mut subscriptions = usecase.subscriptions.lock();
    assert!(subscriptions.get("old").is_err());
    assert!(subscriptions.get("current").is_err());
    for _ in 0..TERMINAL_ATTACHMENT_LIMIT {
        subscriptions.reserve_attachment().unwrap();
    }
}

#[tokio::test]
async fn test_terminal購読_終了済みは再同期せず途中終了は再同期する() {
    for ending in ["snapshot", "exit", "disconnect"] {
        // Given
        let (usecase, gateway, events) = fixture(ending == "snapshot");
        let mut output = usecase.subscribe("renderer".into()).unwrap();
        output.next().await.unwrap();
        usecase
            .attach("renderer", "terminal".into(), "stream".into(), &owner())
            .unwrap();
        output.next().await.unwrap();
        // When
        if ending == "exit" {
            events.0.lock()[0]
                .send(TerminalSurfaceEvent::Exit {
                    session_key: owner().stable_key(),
                    runtime_generation: 1,
                    exit_code: Some(0),
                    sequence: 1,
                })
                .unwrap();
            assert!(matches!(
                output.next().await,
                Some(TerminalSubscriptionEvent::Item {
                    item: TerminalSurfaceStreamItem::Exit { .. },
                    ..
                })
            ));
        } else if ending == "disconnect" {
            events.0.lock().clear();
        }
        // Then
        assert!(
            matches!(output.next().await, Some(TerminalSubscriptionEvent::Closed { resynchronize, .. }) if resynchronize == (ending == "disconnect"))
        );
        assert_eq!(*gateway.deactivated.lock(), ["terminal"]);
    }
}

#[tokio::test]
async fn test_terminal購読_生成失敗は予約を返し終了した購読へのattachを拒否する() {
    // Given
    let application = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(FakePtyGateway::new()),
        Arc::new(Events::default()),
    ));
    let usecase = TerminalSubscriptionUsecase::new(application);
    let output = usecase.subscribe("renderer".into()).unwrap();
    // When / Then
    for _ in 0..32 {
        assert!(matches!(
            usecase.attach("renderer", "terminal".into(), "stream".into(), &owner()),
            Err(UsecaseError::Application(_))
        ));
    }
    drop(output);
    assert!(matches!(
        usecase.attach("renderer", "terminal".into(), "stream".into(), &owner()),
        Err(UsecaseError::Subscription(TerminalSubscriptionError::Ended))
    ));
    assert!(usecase.subscribe("renderer".into()).is_ok());
}
