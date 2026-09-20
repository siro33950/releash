use std::sync::Mutex;
use std::time::Duration;

use crate::domain::application_lifecycle::{ApplicationLifecycleError, ApplicationShutdownGateway};

pub(crate) const STAGES: [&str; 5] = ["commands", "observer", "terminals", "api", "telemetry"];

#[derive(Default)]
pub(crate) struct FakeShutdown {
    pub(crate) failed: Option<&'static str>,
    pub(crate) blocked: Option<&'static str>,
    pub(crate) delay: Duration,
    pub(crate) calls: Mutex<Vec<&'static str>>,
    pub(crate) failure_id: String,
}

impl FakeShutdown {
    async fn stage(&self, name: &'static str) -> Result<(), ApplicationLifecycleError> {
        self.calls.lock().unwrap().push(name);
        println!("shutdown-stage:{name}");
        tokio::time::sleep(self.delay).await;
        if self.blocked == Some(name) {
            std::future::pending::<()>().await;
        }
        if self.failed == Some(name) {
            return Err(ApplicationLifecycleError(format!(
                "{name} failed {}",
                self.failure_id
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ApplicationShutdownGateway for FakeShutdown {
    async fn stop_commands(&self) -> Result<(), ApplicationLifecycleError> {
        self.stage("commands").await
    }
    async fn stop_provider_exit_observer(&self) -> Result<(), ApplicationLifecycleError> {
        self.stage("observer").await
    }
    async fn save_terminals(&self) -> Result<(), ApplicationLifecycleError> {
        self.stage("terminals").await
    }
    async fn stop_local_api(&self) -> Result<(), ApplicationLifecycleError> {
        self.stage("api").await
    }
    async fn shutdown_telemetry(&self) -> Result<(), ApplicationLifecycleError> {
        self.stage("telemetry").await
    }
    async fn wait_for_deadline(&self, duration: Duration) {
        assert_eq!(duration, Duration::from_secs(15));
        tokio::time::sleep(duration).await;
        println!("shutdown-deadline:15");
    }
}
