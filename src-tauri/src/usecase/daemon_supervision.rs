use super::app_config::query_service::DesktopSettingsDto;
use super::client_connection::{
    ClientConnectionDto, ClientConnectionError, ClientConnectionQueryService,
};
use crate::domain::client_operation::transmission::{TransmissionFailure, WriteProgress};
use crate::domain::daemon_supervision::{
    ClientOperation, DaemonSupervision, Failure, FailureStage, Phase, ShellOperation, StopIntent,
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct DaemonSupervisionError(pub String);
impl From<String> for DaemonSupervisionError {
    fn from(reason: String) -> Self {
        Self(reason)
    }
}
impl From<&str> for DaemonSupervisionError {
    fn from(reason: &str) -> Self {
        Self(reason.into())
    }
}
impl From<DaemonSupervisionError> for String {
    fn from(error: DaemonSupervisionError) -> Self {
        error.0
    }
}

#[derive(Clone)]
pub(crate) struct DaemonConnection {
    pub connected_at_ms: u64,
    pub endpoint: ClientConnectionDto,
    pub settings: DesktopSettingsDto,
    pub launch_id: String,
    pub release: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct DesktopSendError {
    #[serde(serialize_with = "serialize_transmission_failure")]
    pub state: TransmissionFailure,
    #[serde(rename = "message")]
    pub reason: String,
}
impl DesktopSendError {
    pub fn not_sent(reason: impl ToString) -> Self {
        Self {
            state: WriteProgress::NotStarted.failure(),
            reason: reason.to_string(),
        }
    }
    pub fn write_attempted(reason: impl ToString) -> Self {
        Self {
            state: WriteProgress::Attempted.failure(),
            reason: reason.to_string(),
        }
    }
}
fn serialize_transmission_failure<S: serde::Serializer>(
    state: &TransmissionFailure,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(match state {
        TransmissionFailure::NotSent => "not_sent",
        TransmissionFailure::Unknown => "unknown",
    })
}

pub(crate) struct DesktopAttachment {
    pub hello: Vec<u8>,
    pub frames: tokio::sync::broadcast::Receiver<Option<Vec<u8>>>,
    pub cancelled: tokio::sync::oneshot::Receiver<()>,
}

#[async_trait::async_trait]
pub(crate) trait DaemonGateway:
    crate::domain::daemon_supervision::DaemonProcessPort
{
    async fn connection(&self) -> Result<Option<DaemonConnection>, Failure>;
    fn connected(&self) -> bool;
    fn restored(&self) -> bool;
    async fn finish_restoration(&self, attachment_id: &str) -> Result<(), String>;
    fn attach(&self, id: String) -> Result<DesktopAttachment, String>;
    fn detach(&self, id: &str);
    async fn forward(&self, bytes: Vec<u8>) -> Result<(), DesktopSendError>;
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonStatus {
    pub connection_generation: u64,
    pub phase: &'static str,
    pub stop_intent: Option<&'static str>,
    pub stage: Option<&'static str>,
    pub reason: Option<String>,
    pub retries: usize,
    pub retry_available: bool,
}

struct State {
    supervision: DaemonSupervision,
    connection: Option<DaemonConnection>,
}

pub(crate) struct DaemonSupervisionUsecase {
    state: parking_lot::Mutex<State>,
    gateway: Arc<dyn DaemonGateway>,
    commands: tokio::sync::mpsc::UnboundedSender<Control>,
    changes: tokio::sync::watch::Sender<DaemonStatus>,
}

enum Control {
    Retry,
    Stop(StopIntent),
    StopFailed(String),
    ShutdownResponse(crate::domain::daemon_supervision::ShutdownResponse),
    SwitchFailed(FailureStage, String),
}

impl DaemonSupervisionUsecase {
    pub fn start(gateway: Arc<dyn DaemonGateway>) -> Arc<Self> {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let state = State {
            supervision: DaemonSupervision::new(gateway.monotonic_ms()),
            connection: None,
        };
        let (changes, _) = tokio::sync::watch::channel(snapshot(&state.supervision));
        let this = Arc::new(Self {
            state: parking_lot::Mutex::new(state),
            gateway: gateway.clone(),
            commands,
            changes,
        });
        tokio::spawn(this.clone().run(gateway, receiver));
        this
    }
    pub fn status(&self) -> DaemonStatus {
        snapshot(&self.state.lock().supervision)
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<DaemonStatus> {
        self.changes.subscribe()
    }
    pub fn retry(&self) -> Result<(), DaemonSupervisionError> {
        if !self.state.lock().supervision.retry_available() {
            return Err("Daemon retry is unavailable.".into());
        }
        self.commands
            .send(Control::Retry)
            .map_err(|e| DaemonSupervisionError(e.to_string()))
    }
    pub fn stop(&self, intent: StopIntent) -> Result<(), DaemonSupervisionError> {
        let accepted = {
            let mut state = self.state.lock();
            let accepted = state.supervision.begin_stop(intent);
            state
                .supervision
                .arm_quit_deadline(self.gateway.monotonic_ms());
            accepted
        };
        self.publish();
        if accepted {
            self.commands
                .send(Control::Stop(intent))
                .map_err(|e| DaemonSupervisionError(e.to_string()))
        } else {
            Ok(())
        }
    }
    pub fn restart_failed(&self, reason: String) -> Result<(), DaemonSupervisionError> {
        self.switch_failed(FailureStage::Restart, reason)
    }
    fn switch_failed(
        &self,
        stage: FailureStage,
        reason: String,
    ) -> Result<(), DaemonSupervisionError> {
        self.commands
            .send(Control::SwitchFailed(stage, reason))
            .map_err(|e| DaemonSupervisionError(e.to_string()))
    }
    pub fn validate_connection(
        &self,
        launch_id: &str,
        release: &str,
    ) -> Result<(), DaemonSupervisionError> {
        let expected = self
            .connection()
            .map_err(|e| DaemonSupervisionError(e.to_string()))?;
        crate::domain::daemon_supervision::verify_identity(&expected.launch_id, launch_id, release)
            .map_err(|failure| DaemonSupervisionError(failure.reason))?;
        Ok(())
    }
    pub fn connection(&self) -> Result<DaemonConnection, ClientConnectionError> {
        let state = self.state.lock();
        if !state.supervision.connection_admitted() {
            return Err(ClientConnectionError("Daemon is not ready.".into()));
        }
        state
            .connection
            .clone()
            .ok_or_else(|| ClientConnectionError("Daemon connection is unavailable.".into()))
    }
    pub fn desktop_action(
        &self,
        hidden: bool,
        first_ready: bool,
        has_failure_window: bool,
    ) -> crate::domain::daemon_supervision::DesktopAction {
        self.state
            .lock()
            .supervision
            .desktop_action(hidden, first_ready, has_failure_window)
    }
    pub fn admit_client_command(
        &self,
        operation: ClientOperation,
    ) -> Result<(), DaemonSupervisionError> {
        if self.gateway.connected()
            && self
                .state
                .lock()
                .supervision
                .client_command_admitted(operation)
        {
            Ok(())
        } else {
            Err("Releash is starting or switching; this request was not accepted.".into())
        }
    }
    pub fn attach(&self, id: String) -> Result<DesktopAttachment, DaemonSupervisionError> {
        self.connection()
            .map_err(|e| DaemonSupervisionError(e.to_string()))?;
        self.state
            .lock()
            .supervision
            .begin_restoration(self.gateway.monotonic_ms());
        self.gateway.attach(id).map_err(DaemonSupervisionError)
    }
    pub async fn finish_restoration(
        &self,
        launch_id: &str,
        attachment_id: &str,
        generation: u64,
    ) -> Result<(), DaemonSupervisionError> {
        self.validate_connection(launch_id, env!("CARGO_PKG_VERSION"))?;
        if !self
            .state
            .lock()
            .supervision
            .restoration_current(generation)
        {
            return Err("Desktop restoration attempt is no longer current.".into());
        }
        if let Err(reason) = self.gateway.finish_restoration(attachment_id).await {
            self.fail_restoration(generation, reason.clone());
            return Err(reason.into());
        }
        let completed = self.gateway.restored()
            && self
                .state
                .lock()
                .supervision
                .finish_restoration(generation, self.gateway.monotonic_ms());
        self.publish();
        if completed {
            Ok(())
        } else {
            Err("Desktop restoration attempt is no longer current.".into())
        }
    }
    pub fn fail_restoration(&self, generation: u64, reason: String) {
        self.state
            .lock()
            .supervision
            .fail_restoration(generation, reason);
        self.publish();
    }
    pub fn detach(&self, id: &str) {
        self.gateway.detach(id);
    }
    pub async fn send_frame(
        &self,
        launch_id: &str,
        operation: Option<ClientOperation>,
        bytes: Vec<u8>,
    ) -> Result<(), DesktopSendError> {
        self.validate_connection(launch_id, env!("CARGO_PKG_VERSION"))
            .map_err(DesktopSendError::not_sent)?;
        if let Some(operation) = operation {
            self.admit_client_command(operation)
                .map_err(DesktopSendError::not_sent)?;
        }
        self.gateway.forward(bytes).await
    }
    pub fn command_admitted(&self, operation: ShellOperation) -> bool {
        self.state
            .lock()
            .supervision
            .shell_command_admitted(operation, self.gateway.connected())
    }
    pub async fn wait_for_update_stop(&self) -> Result<(), DaemonSupervisionError> {
        self.stop(StopIntent::Update)?;
        let mut changes = self.subscribe();
        loop {
            changes.borrow_and_update();
            if self.state.lock().supervision.update_stop_result()? {
                return Ok(());
            }
            changes
                .changed()
                .await
                .map_err(|e| DaemonSupervisionError(e.to_string()))?;
        }
    }
    pub fn begin_update_install(&self) -> Result<(), DaemonSupervisionError> {
        if !self.state.lock().supervision.begin_update_install() {
            return Err("Update cannot begin before coordinated shutdown completes.".into());
        }
        self.publish();
        Ok(())
    }
    pub fn finish_update_install(&self, failure: Option<String>) -> bool {
        let restart = {
            let mut state = self.state.lock();
            state.supervision.finish_update_install(failure)
        };
        self.publish();
        restart
    }
    fn publish(&self) {
        self.changes.send_if_modified(|current| {
            let next = self.status();
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        });
    }
    async fn run(
        self: Arc<Self>,
        gateway: Arc<dyn DaemonGateway>,
        mut commands: tokio::sync::mpsc::UnboundedReceiver<Control>,
    ) {
        let now = || gateway.monotonic_ms();
        let mut launch_id = String::new();
        let mut child_running = false;
        let mut spawn = true;
        let mut last_connection_error = None;
        loop {
            tokio::select! {
                biased;
                command = commands.recv() => match command {
                    Some(Control::Retry) => {
                        if child_running {
                            self.state.lock().supervision.retry_restoration(now());
                        } else if self.state.lock().supervision.retry(now()) {
                            spawn = true;
                        }
                    }
                    Some(Control::Stop(intent)) => {
                        {
                            spawn = false;
                            if !child_running {
                                self.state.lock().supervision.uncoordinated_stop();
                            } else if self.state.lock().connection.is_none() {
                                match gateway.terminate_and_wait().await {
                                    Ok(()) => { child_running = false; self.state.lock().supervision.uncoordinated_stop(); }
                                    Err(reason) => self.state.lock().supervision.stop_failed(FailureStage::Shutdown, reason),
                                }
                            } else {
                                let gateway = gateway.clone();
                                let sender = self.commands.clone();
                                tokio::spawn(async move {
                                    let response = match gateway.request_shutdown(intent).await {
                                        Ok(response) => Control::ShutdownResponse(response),
                                        Err(reason) => Control::StopFailed(reason),
                                    };
                                    let _ = sender.send(response);
                                });
                            }
                        }
                    }
                    Some(Control::ShutdownResponse(response)) => self.state.lock().supervision.shutdown_response(response),
                    Some(Control::StopFailed(reason)) => {
                        let mut state = self.state.lock();
                        state.supervision.stop_failed(FailureStage::Shutdown, reason);
                    }
                    Some(Control::SwitchFailed(stage, reason)) => self.state.lock().supervision.stop_failed(stage, reason),
                    None => return,
                },
                _ = std::future::ready(()), if spawn => {
                spawn = false;
                last_connection_error = None;
                match gateway.spawn().await {
                    Ok(id) => {
                        launch_id = id;
                        child_running = true;
                    }
                    Err(reason) => self.state.lock().supervision.spawn_failed(reason, now()),
                }
                },
                _ = gateway.wait_for_poll() => {
                    self.state.lock().supervision.arm_quit_deadline(now());
                    if self.state.lock().supervision.quit_expired(now()) {
                        let result = gateway.terminate_and_wait().await;
                        if let Err(reason) = &result { log::error!("{reason}"); }
                        self.state.lock().supervision.finish_quit_termination(result);
                        self.publish();
                        return;
                    }
                    if child_running {
                        match gateway.exited().await {
                            Ok(Some(exit)) => {
                                child_running = false;
                                let mut state = self.state.lock();
                                state.connection = None;
                                state.supervision.observe_exit(exit, now());
                            }
                            Err(reason) => self.state.lock().supervision.stop_failed(FailureStage::UnexpectedExit, reason),
                            Ok(None) => {}
                        }
                    }
                    if !gateway.connected() { self.state.lock().supervision.connection_lost(now()); }
                    let phase = self.state.lock().supervision.phase();
                    if child_running && phase == Phase::Starting {
                        match gateway.connection().await {
                            Ok(Some(connection)) => match crate::domain::daemon_supervision::verify_identity(&launch_id, &connection.launch_id, &connection.release) {
                                Ok(()) => { let mut state = self.state.lock(); if state.supervision.connected(connection.connected_at_ms) { state.connection = Some(connection); } }
                                Err(failure) => last_connection_error = Some(failure),
                            },
                            Ok(None) => {},
                            Err(failure) => last_connection_error = Some(failure),
                        }
                        let interruption = self.state.lock().supervision.startup_interruption(now(), last_connection_error.clone());
                        if let Some(interruption) = interruption {
                            self.state.lock().connection = None;
                            self.publish();
                            match gateway.terminate_and_wait().await {
                                Ok(()) => {
                                    child_running = false;
                                    self.state.lock().supervision.startup_terminated(interruption, now());
                                }
                                Err(reason) => self.state.lock().supervision.stop_failed(FailureStage::Shutdown, reason),
                            }
                        }
                    }
                    self.state.lock().supervision.expire_restoration(now());
                    if !child_running { spawn = self.state.lock().supervision.restart_due(now()); }
                }
            }
            self.publish();
        }
    }
}

fn snapshot(supervision: &DaemonSupervision) -> DaemonStatus {
    DaemonStatus {
        connection_generation: supervision.connection_generation(),
        stop_intent: supervision.stop_intent().map(|intent| match intent {
            StopIntent::Quit(_) => "quit",
            StopIntent::Restart => "restart",
            StopIntent::Update => "update",
        }),
        phase: match supervision.phase() {
            Phase::Starting => "starting",
            Phase::Ready => "ready",
            Phase::Restoring => "restoring",
            Phase::Backoff => "backoff",
            Phase::Failed => "failed",
            Phase::Stopping => "stopping",
            Phase::Installing => "installing",
            Phase::Stopped => "stopped",
        },
        stage: supervision.failure().map(|f| match f.stage {
            FailureStage::Spawn => "spawn",
            FailureStage::Initialization => "backend_initialization",
            FailureStage::StartupTimeout => "startup_timeout",
            FailureStage::UnexpectedExit => "unexpected_exit",
            FailureStage::Identity => "connection_identity",
            FailureStage::Shutdown => "shutdown",
            FailureStage::Update => "update",
            FailureStage::Restart => "restart",
            FailureStage::Restoration => "state_restoration",
        }),
        reason: supervision.failure().map(|f| f.reason.clone()),
        retries: supervision.retries(),
        retry_available: supervision.retry_available(),
    }
}

#[async_trait::async_trait]
impl ClientConnectionQueryService for Arc<DaemonSupervisionUsecase> {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        Ok(self.connection()?.endpoint)
    }
    async fn desktop_settings(&self) -> Result<DesktopSettingsDto, ClientConnectionError> {
        Ok(self.connection()?.settings)
    }
}

#[cfg(test)]
#[path = "daemon_supervision_test.rs"]
mod daemon_supervision_tests;
