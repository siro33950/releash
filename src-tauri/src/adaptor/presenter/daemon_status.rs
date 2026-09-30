use crate::usecase::daemon_supervision::{
    DaemonStatus, DaemonStatusOutput, DaemonStatusSubscriptionDriver,
};

pub(crate) struct DaemonStatusPresenter {
    channel: tauri::ipc::Channel<DaemonStatusMessage>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DaemonStatusMessage {
    connection_generation: u64,
    phase: &'static str,
    stop_intent: Option<&'static str>,
    stage: Option<&'static str>,
    reason: Option<String>,
    retries: usize,
    retry_available: bool,
}

impl From<DaemonStatus> for DaemonStatusMessage {
    fn from(status: DaemonStatus) -> Self {
        Self {
            connection_generation: status.connection_generation,
            phase: status.phase,
            stop_intent: status.stop_intent,
            stage: status.stage,
            reason: status.reason,
            retries: status.retries,
            retry_available: status.retry_available,
        }
    }
}

impl DaemonStatusPresenter {
    pub fn new(channel: tauri::ipc::Channel<DaemonStatusMessage>) -> Self {
        Self { channel }
    }
}

impl DaemonStatusOutput for DaemonStatusPresenter {
    fn send(&self, status: DaemonStatus) -> bool {
        crate::infrastructure::desktop_channel::send(&self.channel, status.into())
    }
}

pub(crate) struct DaemonStatusDriver;

impl DaemonStatusSubscriptionDriver for DaemonStatusDriver {
    fn start(
        &self,
        id: String,
        task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    ) {
        crate::infrastructure::desktop_channel::start(id, task);
    }

    fn stop(&self, id: &str) {
        crate::infrastructure::desktop_channel::stop(id);
    }
}
