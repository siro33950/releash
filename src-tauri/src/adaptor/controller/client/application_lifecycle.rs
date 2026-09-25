use crate::adaptor::presenter::error::AppError;
#[path = "application_lifecycle_shared.rs"]
mod shared;
use crate::adaptor::protocol::application_lifecycle_v1::{
    ApplicationQuitIntentDtoV1, ApplicationQuitOutcomeDtoV1, ApplicationQuitRequestDtoV1,
    ApplicationStartupOutcomeDtoV1, StartupFailureQuitOutcomeDtoV1,
};
use crate::domain::application_lifecycle::ApplicationQuitIntent;
pub(crate) use shared::register_shared;
use std::sync::Arc;

pub(crate) fn get_application_startup_outcome_shared(
    authority: &Arc<crate::usecase::application_startup::ApplicationStartupAuthority>,
) -> ApplicationStartupOutcomeDtoV1 {
    crate::adaptor::presenter::application_lifecycle::application_startup_outcome(
        authority.outcome(),
    )
}

pub(crate) fn quit_after_startup_failure_shared(
    authority: &Arc<crate::usecase::application_startup::ApplicationStartupAuthority>,
) -> Result<
    StartupFailureQuitOutcomeDtoV1,
    crate::usecase::application_startup::ApplicationUnavailable,
> {
    let correlation_id = authority.quit_after_failure()?;
    Ok(StartupFailureQuitOutcomeDtoV1::Accepted { correlation_id })
}

pub(crate) fn request_application_quit_shared(
    process_port: &dyn crate::domain::application_lifecycle::ApplicationQuitIntentPort,
    request: ApplicationQuitRequestDtoV1,
) -> Result<ApplicationQuitOutcomeDtoV1, AppError> {
    let intent = match request.intent {
        ApplicationQuitIntentDtoV1::Exit { code } => ApplicationQuitIntent::Exit { code },
        ApplicationQuitIntentDtoV1::Restart { code } => ApplicationQuitIntent::Restart { code },
    };
    crate::usecase::application_lifecycle::request_quit(process_port, intent)
        .map_err(AppError::from_failure)?;
    Ok(ApplicationQuitOutcomeDtoV1::Accepted)
}

#[cfg(test)]
#[path = "application_lifecycle_test.rs"]
mod application_lifecycle_tests;
