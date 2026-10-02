use crate::usecase::state_subscription::{
    StateReadError, StateValue, SubscriptionError, SubscriptionTarget,
};
use crate::usecase::terminal_surface::subscription::{
    TerminalSubscriptionOutput, TerminalSubscriptionUsecase,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Output {
    snapshots: AtomicUsize,
    entered: tokio::sync::Notify,
    completed: tokio::sync::Notify,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    reset: parking_lot::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
impl TerminalSubscriptionOutput for Output {
    fn start(
        &self,
        _: &str,
        _: &SubscriptionTarget,
        _: &str,
        _: Option<(&str, u64)>,
    ) -> Result<Option<usize>, StateReadError> {
        Ok(Some(6000))
    }
    fn stop(&self, _: &str, _: &SubscriptionTarget, _: &str) -> Result<(), SubscriptionError> {
        Ok(())
    }
    fn set_snapshot(
        &self,
        _: &SubscriptionTarget,
        _: u64,
        _: u64,
        _: StateValue,
    ) -> Result<(), SubscriptionError> {
        if let Some(reset) = self.reset.lock().as_ref() {
            reset();
        }
        let snapshot = self.snapshots.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if snapshot == 0 {
            self.release.lock().unwrap().recv().unwrap();
        }
        self.completed.notify_one();
        Ok(())
    }
    fn publish_failure(
        &self,
        _: &SubscriptionTarget,
        _: StateReadError,
    ) -> Result<(), SubscriptionError> {
        panic!("unexpected failure")
    }
}
async fn assert_stopped(close: bool) {
    // Given
    let (terminal, _, _, surface) =
        crate::test_support::state_subscription::terminal_application_fixture();
    let (release, released) = std::sync::mpsc::channel();
    let output = Arc::new(Output {
        snapshots: AtomicUsize::new(0),
        entered: Default::default(),
        completed: Default::default(),
        release: std::sync::Mutex::new(released),
        reset: Default::default(),
    });
    let (requests, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let usecase = TerminalSubscriptionUsecase::new(output.clone(), Some(terminal), requests);
    let target = SubscriptionTarget::Terminal(surface.owner);
    usecase.open_client("client".into()).unwrap();
    usecase
        .start_terminal("client", &target, "input", None)
        .await
        .unwrap();
    *output.reset.lock() = Some(Box::new({
        let usecase = usecase.clone();
        let target = target.clone();
        move || {
            crate::usecase::terminal_surface::subscription::subscription_tests::add_reset(
                &usecase, &target, "client",
            )
        }
    }));
    usecase.schedule_terminal_refresh(vec!["client".into()], target.clone());
    let mut worker = super::spawn_worker(receiver.recv().await.unwrap());
    output.entered.notified().await;
    let snapshots = output.snapshots.load(Ordering::SeqCst);
    assert_eq!(snapshots, 1);
    // When
    if close {
        usecase.close_client("client");
    } else {
        usecase
            .stop_subscription("client", &target, "input")
            .unwrap();
    }
    crate::usecase::terminal_surface::subscription::subscription_tests::add_reset(
        &usecase, &target, "client",
    );
    release.send(()).unwrap();
    output.completed.notified().await;
    tokio::select! {
        result = &mut worker => result.unwrap(),
        _ = output.entered.notified() => {
            worker.abort();
            output.reset.lock().take();
            panic!("stopped worker produced another snapshot");
        }
    }
    // Then
    assert_eq!(output.snapshots.load(Ordering::SeqCst), snapshots);
    assert_eq!(usecase.test_worker_count(), 0);
    output.reset.lock().take();
}
#[tokio::test]
async fn test_terminal駆動_最終購読者の停止後は作り直しを続けない() {
    assert_stopped(false).await;
}
#[tokio::test]
async fn test_terminal駆動_最終clientの切断後は作り直しを続けない() {
    assert_stopped(true).await;
}
