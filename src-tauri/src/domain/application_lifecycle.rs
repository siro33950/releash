pub(crate) const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplicationQuitIntent {
    Exit { code: i32 },
    Restart { code: i32 },
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct ApplicationLifecycleError(pub String);

pub(crate) trait ApplicationQuitIntentPort: Send + Sync {
    fn execute(&self, intent: ApplicationQuitIntent) -> Result<(), ApplicationLifecycleError>;
}

#[async_trait::async_trait]
pub(crate) trait ApplicationShutdownGateway: Send + Sync {
    async fn stop_commands(&self) -> Result<(), ApplicationLifecycleError>;
    async fn stop_provider_exit_observer(&self) -> Result<(), ApplicationLifecycleError>;
    async fn save_terminals(&self) -> Result<(), ApplicationLifecycleError>;
    async fn stop_local_api(&self) -> Result<(), ApplicationLifecycleError>;
    async fn shutdown_telemetry(&self) -> Result<(), ApplicationLifecycleError>;
    async fn wait_for_deadline(&self, duration: std::time::Duration);
}

#[derive(Default)]
pub(crate) struct CommandAdmission {
    stopped: bool,
}

impl CommandAdmission {
    pub(crate) fn accepts_start(&self) -> bool {
        !self.stopped
    }

    pub(crate) fn accepts_completion(&self) -> bool {
        !self.stopped
    }

    pub(crate) fn stop(&mut self) {
        self.stopped = true;
    }
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;
