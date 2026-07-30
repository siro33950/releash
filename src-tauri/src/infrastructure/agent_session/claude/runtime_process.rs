use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::infrastructure::agent_session::claude::process::ClaudeStdioHandle;

pub(crate) struct ClaudeRuntimeProcess {
    pub(crate) handle: ClaudeStdioHandle,
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) read_task: Option<tokio::task::JoinHandle<()>>,
}

impl ClaudeRuntimeProcess {
    pub(crate) async fn shutdown(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        self.handle.shutdown().await;
        if let Some(read_task) = self.read_task.take() {
            if let Err(error) = read_task.await {
                log::warn!("failed to join Claude stdout reader: {error}");
            }
        }
    }
}
