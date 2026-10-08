use crate::adaptor::presenter::error::AppError;
#[path = "application_lifecycle_shared.rs"]
pub(crate) mod shared;
use crate::domain::daemon::{StopAcceptance, StopRequest};
pub(crate) use shared::register_shared;

pub(crate) async fn stop_daemon_shared(
    daemon: &crate::usecase::daemon::DaemonUsecase,
    process_port: &tokio::sync::mpsc::Sender<i32>,
) -> Result<(), AppError> {
    if let StopAcceptance::Started { code } = daemon.stop(StopRequest::Exit { code: 0 }).await {
        process_port.try_send(code).map_err(|error| {
            AppError::from_failure(
                crate::domain::application_lifecycle::ApplicationLifecycleError(format!(
                    "daemon exit request could not be accepted: {error}"
                )),
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;
