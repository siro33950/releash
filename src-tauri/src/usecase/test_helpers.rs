use crate::domain::daemon_supervision::{
    DaemonExit, DaemonProcessPort, Failure, ShutdownResponse, StopIntent,
};
use crate::usecase::{
    app_config::query_service::DesktopSettingsDto,
    client_connection::ClientConnectionDto,
    daemon_supervision::{DaemonConnection, DaemonGateway},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
pub(crate) struct FakeDaemon {
    origin: std::sync::OnceLock<tokio::time::Instant>,
    pub(crate) starts: AtomicUsize,
    pub(crate) ready: AtomicBool,
    pub(crate) connection_delay_ms: AtomicUsize,
    pub(crate) connection_delivery_delay_ms: AtomicUsize,
    pub(crate) start_minimized: AtomicBool,
    pub(crate) shutdown_response: parking_lot::Mutex<Option<Result<ShutdownResponse, String>>>,
    pub(crate) termination_error: parking_lot::Mutex<Option<String>>,
    pub(crate) spawn_failure: AtomicBool,
    pub(crate) wrong_identity: AtomicBool,
    pub(crate) exit: parking_lot::Mutex<Option<DaemonExit>>,
    pub(crate) calls: parking_lot::Mutex<Vec<&'static str>>,
    pub(crate) restoration_error: parking_lot::Mutex<Option<String>>,
    pub(crate) restoration_wait: parking_lot::Mutex<Option<std::sync::Arc<tokio::sync::Notify>>>,
}
#[async_trait::async_trait]
impl DaemonProcessPort for FakeDaemon {
    fn monotonic_ms(&self) -> u64 {
        self.origin
            .get_or_init(tokio::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
    async fn wait_for_poll(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    async fn spawn(&self) -> Result<String, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.calls.lock().push("spawn");
        if self.spawn_failure.load(Ordering::SeqCst) {
            Err("executable missing".into())
        } else {
            Ok("launch".into())
        }
    }
    async fn exited(&self) -> Result<Option<DaemonExit>, String> {
        Ok(self.exit.lock().take())
    }
    async fn terminate_and_wait(&self) -> Result<(), String> {
        self.calls.lock().push("terminate_and_wait");
        self.termination_error.lock().clone().map_or(Ok(()), Err)
    }
    async fn request_shutdown(&self, _intent: StopIntent) -> Result<ShutdownResponse, String> {
        self.calls.lock().push("shutdown");
        self.shutdown_response
            .lock()
            .clone()
            .unwrap_or(Ok(ShutdownResponse::Accepted))
    }
}
#[async_trait::async_trait]
impl DaemonGateway for FakeDaemon {
    fn attach(&self, _: String) -> Result<super::daemon_supervision::DesktopAttachment, String> {
        Err("No fake desktop attachment".into())
    }
    fn detach(&self, _: &str) {}
    async fn forward(&self, _: Vec<u8>) -> Result<(), super::daemon_supervision::DesktopSendError> {
        self.calls.lock().push("forward");
        Ok(())
    }
    fn restored(&self) -> bool {
        self.connected()
    }
    async fn finish_restoration(&self, _: &str) -> Result<(), String> {
        let wait = self.restoration_wait.lock().clone();
        if let Some(wait) = wait {
            wait.notified().await;
        }
        self.restoration_error.lock().clone().map_or(Ok(()), Err)
    }
    fn connected(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
    async fn connection(&self) -> Result<Option<DaemonConnection>, Failure> {
        if !self.ready.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let delay = self.connection_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
        }
        let connection = DaemonConnection {
            connected_at_ms: self.monotonic_ms(),
            endpoint: ClientConnectionDto {
                url: "ws://127.0.0.1:1".into(),
                auth_subprotocol: "client-only".into(),
            },
            settings: DesktopSettingsDto {
                close_to_tray: true,
                start_minimized: self.start_minimized.load(Ordering::SeqCst),
                crash_reporting: false,
                performance_telemetry: false,
            },
            launch_id: if self.wrong_identity.load(Ordering::SeqCst) {
                "wrong"
            } else {
                "launch"
            }
            .into(),
            release: env!("CARGO_PKG_VERSION").into(),
        };
        let delay = self.connection_delivery_delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
        }
        Ok(Some(connection))
    }
}
pub(crate) async fn restore_desktop(
    supervisor: &super::daemon_supervision::DaemonSupervisionUsecase,
) {
    supervisor
        .finish_restoration(
            "launch",
            "desktop",
            supervisor.status().connection_generation,
        )
        .await
        .unwrap();
}
pub(crate) async fn tick(milliseconds: u64) {
    for _ in 0..milliseconds.div_ceil(100) {
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
    }
}
