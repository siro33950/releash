use crate::adaptor::presenter::error::AppError;
#[path = "application_lifecycle_shared.rs"]
pub(crate) mod shared;
use crate::adaptor::presenter::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitOutcomeDtoV1, ApplicationQuitRequestDtoV1,
};
use crate::domain::daemon::{StopAcceptance, StopRequest};
pub(crate) use shared::register_shared;

pub(crate) async fn request_application_quit_shared(
    daemon: &crate::usecase::daemon::DaemonUsecase,
    process_port: &tokio::sync::mpsc::Sender<i32>,
    request: ApplicationQuitRequestDtoV1,
) -> Result<ApplicationQuitOutcomeDtoV1, AppError> {
    let code = match request.intent {
        ApplicationQuitIntentDtoV1::Exit { code }
        | ApplicationQuitIntentDtoV1::Restart { code } => code,
    };
    if let StopAcceptance::Started { code } = daemon.stop(StopRequest::Exit { code }).await {
        process_port.try_send(code).map_err(|error| {
            AppError::from_failure(
                crate::domain::application_lifecycle::ApplicationLifecycleError(format!(
                    "daemon exit request could not be accepted: {error}"
                )),
            )
        })?;
    }
    Ok(ApplicationQuitOutcomeDtoV1::Accepted)
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;
