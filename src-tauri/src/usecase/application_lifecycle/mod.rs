use crate::domain::application_lifecycle::{
    ApplicationLifecycleError, ApplicationQuitIntent, ApplicationQuitIntentPort,
    ApplicationShutdownGateway, SHUTDOWN_TIMEOUT,
};

pub(crate) fn request_quit(
    port: &dyn ApplicationQuitIntentPort,
    intent: ApplicationQuitIntent,
) -> Result<(), ApplicationLifecycleError> {
    port.execute(intent)
}

pub(crate) async fn shutdown(gateway: &dyn ApplicationShutdownGateway) {
    let cleanup = async {
        if let Err(error) = gateway.stop_commands().await {
            log::error!("application shutdown: command stop failed: {error}");
        }
        if let Err(error) = gateway.stop_provider_exit_observer().await {
            log::error!("application shutdown: provider exit observer stop failed: {error}");
        }
        if let Err(error) = gateway.save_terminals().await {
            log::error!("application shutdown: terminal state save failed: {error}");
        }
        if let Err(error) = gateway.stop_local_api().await {
            log::error!("application shutdown: local API stop failed: {error}");
        }
        if let Err(error) = gateway.shutdown_telemetry().await {
            log::error!("application shutdown: telemetry shutdown failed: {error}");
        }
    };
    tokio::select! {
        biased;
        () = gateway.wait_for_deadline(SHUTDOWN_TIMEOUT) => {
            log::error!("application shutdown: 15 second deadline exceeded; exiting");
        }
        () = cleanup => {}
    }
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;

#[cfg(test)]
pub(crate) mod test_helpers;
