use crate::usecase::daemon_supervision::{DaemonStatus, DaemonStatusOutput};

pub(crate) struct DaemonStatusPresenter {
    channels: crate::infrastructure::desktop_channel::DesktopChannel<DaemonStatusMessage>,
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
    pub fn new() -> Self {
        Self {
            channels: crate::infrastructure::desktop_channel::DesktopChannel::new(),
        }
    }

    pub fn register(&self, id: String, channel: tauri::ipc::Channel<DaemonStatusMessage>) {
        self.channels.register(id, channel);
    }
}

impl DaemonStatusOutput for DaemonStatusPresenter {
    fn start(&self, id: String, status: DaemonStatus) {
        self.channels.send(&id, status.into());
    }

    fn stop(&self, id: &str) {
        self.channels.stop(id);
    }

    fn publish(&self, status: DaemonStatus) {
        self.channels.publish(status.into());
    }
}
