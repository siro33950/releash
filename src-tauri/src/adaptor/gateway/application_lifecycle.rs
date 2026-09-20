use std::sync::Arc;

use crate::domain::application_lifecycle::{
    ApplicationLifecycleError, ApplicationQuitIntent, ApplicationQuitIntentPort,
    ApplicationShutdownGateway,
};

pub(crate) struct DaemonProcessActionPort(pub(crate) tokio::sync::mpsc::Sender<i32>);

impl ApplicationQuitIntentPort for DaemonProcessActionPort {
    fn execute(&self, action: ApplicationQuitIntent) -> Result<(), ApplicationLifecycleError> {
        let code = match action {
            ApplicationQuitIntent::Exit { code } | ApplicationQuitIntent::Restart { code } => code,
        };
        self.0.try_send(code).map_err(|error| {
            ApplicationLifecycleError(format!(
                "daemon exit request could not be accepted: {error}"
            ))
        })
    }
}

#[cfg(all(debug_assertions, feature = "desktop"))]
pub(crate) struct TauriApplicationQuitIntentPort<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

#[cfg(all(debug_assertions, feature = "desktop"))]
impl<R: tauri::Runtime> TauriApplicationQuitIntentPort<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

#[cfg(all(debug_assertions, feature = "desktop"))]
impl<R: tauri::Runtime> crate::domain::application_lifecycle::ApplicationQuitIntentPort
    for TauriApplicationQuitIntentPort<R>
{
    fn execute(
        &self,
        action: ApplicationQuitIntent,
    ) -> Result<(), crate::domain::application_lifecycle::ApplicationLifecycleError> {
        match action {
            ApplicationQuitIntent::Exit { code } => {
                crate::infrastructure::platform::tray::mark_quit_requested();
                self.app.exit(code);
            }
            ApplicationQuitIntent::Restart { .. } => self.app.request_restart(),
        }
        Ok(())
    }
}

pub(crate) struct DaemonShutdownGateway {
    pub(crate) workflow: Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    pub(crate) terminal:
        Arc<crate::usecase::terminal_surface::application::TerminalSurfaceApplication>,
    pub(crate) server: Arc<crate::infrastructure::local_api::LocalApiServer>,
    pub(crate) stop_observer: Arc<dyn Fn() + Send + Sync>,
    pub(crate) telemetry:
        parking_lot::Mutex<Option<crate::infrastructure::telemetry::TelemetryGuard>>,
}

#[async_trait::async_trait]
impl ApplicationShutdownGateway for DaemonShutdownGateway {
    async fn stop_commands(&self) -> Result<(), ApplicationLifecycleError> {
        let workflow = self.workflow.clone();
        tokio::spawn(async move { workflow.shutdown_active_commands().await })
            .await
            .map_err(|error| ApplicationLifecycleError(error.to_string()))
    }

    async fn stop_provider_exit_observer(&self) -> Result<(), ApplicationLifecycleError> {
        let stop = self.stop_observer.clone();
        tokio::task::spawn_blocking(move || stop())
            .await
            .map_err(|error| ApplicationLifecycleError(error.to_string()))
    }

    async fn save_terminals(&self) -> Result<(), ApplicationLifecycleError> {
        let terminal = self.terminal.clone();
        tokio::task::spawn_blocking(move || terminal.shutdown())
            .await
            .map_err(|error| ApplicationLifecycleError(error.to_string()))?
            .map_err(|error| ApplicationLifecycleError(error.to_string()))
    }

    async fn stop_local_api(&self) -> Result<(), ApplicationLifecycleError> {
        let server = self.server.clone();
        tokio::spawn(async move { server.shutdown_and_wait().await })
            .await
            .map_err(|error| ApplicationLifecycleError(error.to_string()))?
            .map_err(|error| ApplicationLifecycleError(error.to_string()))
    }

    async fn shutdown_telemetry(&self) -> Result<(), ApplicationLifecycleError> {
        let telemetry = self.telemetry.lock().take();
        shutdown_telemetry(telemetry).await
    }

    async fn wait_for_deadline(&self, duration: std::time::Duration) {
        tokio::time::sleep(duration).await;
    }
}

async fn shutdown_telemetry(
    telemetry: impl Send + 'static,
) -> Result<(), ApplicationLifecycleError> {
    tokio::task::spawn_blocking(move || drop(telemetry))
        .await
        .map_err(|error| ApplicationLifecycleError(error.to_string()))
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;
