use crate::adaptor::presenter::{connect::ConnectFailure, error::AppError};
use crate::usecase::agent_session::{
    AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecaseError,
    ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::ProviderHookHealthUsecaseError;
#[derive(Clone, Copy)]
pub(crate) enum ProviderParseOperation {
    ConfigureProvider,
    Start,
    ResumeHistory,
}

#[derive(Clone, Copy)]
pub(crate) enum AgentSessionLaunchOperation {
    Start,
    ResumeHistory,
}

#[derive(Clone, Copy)]
pub(crate) enum AgentSessionConflictOperation {
    Start,
    ResumeHistory,
    Update,
}

pub(crate) enum ProviderTuiCodedError {
    ProviderAvailabilityInvalidExecutable,
    ProviderAvailabilityConfigUnavailable,
    ProviderAvailabilityRefreshUnavailable,
    ProviderAvailabilityCorrupt,
    AgentSessionInvalidProvider(ProviderParseOperation),
    AgentSessionProviderUnavailable,
    AgentSessionInvalidInput(AgentSessionLaunchOperation),
    AgentSessionConflict(AgentSessionConflictOperation),
    AgentSessionStorageUnavailable,
    AgentSessionLaunchUnavailable(connectrpc::ErrorCode),
    AgentSessionTerminalUnavailable(connectrpc::ErrorCode),
    AgentSessionCorrupt,
    AgentSessionNotFound,
    AgentSessionInvalidOperation,
    ProviderHookHealthInvalidRequest,
    ProviderHookHealthStorageUnavailable,
    ProviderHookHealthCorrupt,
}

pub(crate) fn provider_tui_coded_error(error: ProviderTuiCodedError) -> AppError {
    let kind = error.connect_code();
    let (code, message) = match error {
        ProviderTuiCodedError::ProviderAvailabilityInvalidExecutable => (
            "PROVIDER_AVAILABILITY_INVALID_EXECUTABLE",
            "Enter a Provider executable command name or path.",
        ),
        ProviderTuiCodedError::ProviderAvailabilityConfigUnavailable => (
            "PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE",
            "Releash could not access the Provider executable setting. Try again.",
        ),
        ProviderTuiCodedError::ProviderAvailabilityRefreshUnavailable => (
            "PROVIDER_AVAILABILITY_REFRESH_UNAVAILABLE",
            "Releash could not refresh Provider CLI availability. Try again.",
        ),
        ProviderTuiCodedError::ProviderAvailabilityCorrupt => (
            "PROVIDER_AVAILABILITY_CORRUPT",
            "Releash could not read Provider CLI availability. Restart Releash and try again.",
        ),
        ProviderTuiCodedError::AgentSessionInvalidProvider(operation) => match operation {
            ProviderParseOperation::ConfigureProvider => {
                ("AGENT_SESSION_INVALID_PROVIDER", "Select a valid Provider.")
            }
            ProviderParseOperation::Start => (
                "AGENT_SESSION_INVALID_PROVIDER",
                "Select a Provider before starting the AgentSession.",
            ),
            ProviderParseOperation::ResumeHistory => (
                "AGENT_SESSION_INVALID_PROVIDER",
                "Select a Provider before resuming the AgentSession.",
            ),
        },
        ProviderTuiCodedError::AgentSessionProviderUnavailable => (
            "AGENT_SESSION_PROVIDER_UNAVAILABLE",
            "The selected Provider is unavailable. Check its executable and try again.",
        ),
        ProviderTuiCodedError::AgentSessionInvalidInput(operation) => match operation {
            AgentSessionLaunchOperation::Start => (
                "AGENT_SESSION_INVALID_INPUT",
                "Releash could not start the AgentSession because the request is invalid.",
            ),
            AgentSessionLaunchOperation::ResumeHistory => (
                "AGENT_SESSION_INVALID_INPUT",
                "Releash could not resume the AgentSession because the request is invalid.",
            ),
        },
        ProviderTuiCodedError::AgentSessionConflict(operation) => match operation {
            AgentSessionConflictOperation::Start => (
                "AGENT_SESSION_CONFLICT",
                "The AgentSession could not be started because the request conflicts with current state or its Provider session is already in use. Refresh and try again.",
            ),
            AgentSessionConflictOperation::ResumeHistory => (
                "AGENT_SESSION_CONFLICT",
                "The AgentSession could not be resumed because it changed or its Provider session is already in use. Refresh and try again.",
            ),
            AgentSessionConflictOperation::Update => (
                "AGENT_SESSION_CONFLICT",
                "The AgentSession could not be updated because it changed or its Provider session is already in use. Refresh and try again.",
            ),
        },
        ProviderTuiCodedError::AgentSessionStorageUnavailable => (
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Releash could not access saved AgentSession data. Try again.",
        ),
        ProviderTuiCodedError::AgentSessionLaunchUnavailable(_) => (
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Releash could not complete the Provider operation for this AgentSession. Try again.",
        ),
        ProviderTuiCodedError::AgentSessionTerminalUnavailable(_) => (
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Releash could not complete the Terminal operation for this AgentSession. Try again.",
        ),
        ProviderTuiCodedError::AgentSessionCorrupt => (
            "AGENT_SESSION_CORRUPT",
            "Releash could not continue because the AgentSession data is invalid.",
        ),
        ProviderTuiCodedError::AgentSessionNotFound => (
            "AGENT_SESSION_NOT_FOUND",
            "The AgentSession is no longer available.",
        ),
        ProviderTuiCodedError::AgentSessionInvalidOperation => (
            "AGENT_SESSION_INVALID_OPERATION",
            "This operation is not available for the AgentSession in its current state. Refresh and try again.",
        ),
        ProviderTuiCodedError::ProviderHookHealthInvalidRequest => (
            "PROVIDER_HOOK_HEALTH_INVALID_REQUEST",
            "Releash could not load Provider Hook health because the request is invalid.",
        ),
        ProviderTuiCodedError::ProviderHookHealthStorageUnavailable => (
            "PROVIDER_HOOK_HEALTH_STORAGE_UNAVAILABLE",
            "Releash could not load Provider Hook health. Try again.",
        ),
        ProviderTuiCodedError::ProviderHookHealthCorrupt => (
            "PROVIDER_HOOK_HEALTH_CORRUPT",
            "Releash could not load Provider Hook health because its saved data is invalid.",
        ),
    };
    AppError::coded(code, message, kind)
}

pub(crate) fn provider_availability_error(error: ProviderAvailabilityUsecaseError) -> AppError {
    match error {
        ProviderAvailabilityUsecaseError::InvalidInput => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderAvailabilityInvalidExecutable)
        }
        ProviderAvailabilityUsecaseError::ConfigUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderAvailabilityConfigUnavailable)
        }
        ProviderAvailabilityUsecaseError::RefreshUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderAvailabilityRefreshUnavailable)
        }
        ProviderAvailabilityUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderAvailabilityCorrupt)
        }
    }
}

pub(crate) fn launch_error(
    error: AgentSessionLaunchUsecaseError,
    operation: AgentSessionLaunchOperation,
) -> AppError {
    let kind = error.connect_code();
    let result = match error {
        AgentSessionLaunchUsecaseError::Technical(stopped) => {
            return AppError::from_failure(stopped)
        }
        AgentSessionLaunchUsecaseError::ProviderUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionProviderUnavailable)
        }
        AgentSessionLaunchUsecaseError::InvalidInput => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionInvalidInput(operation))
        }
        AgentSessionLaunchUsecaseError::Conflict(_) => {
            let operation = match operation {
                AgentSessionLaunchOperation::Start => AgentSessionConflictOperation::Start,
                AgentSessionLaunchOperation::ResumeHistory => {
                    AgentSessionConflictOperation::ResumeHistory
                }
            };
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionConflict(operation))
        }
        AgentSessionLaunchUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionStorageUnavailable)
        }
        AgentSessionLaunchUsecaseError::LaunchUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionLaunchUnavailable(kind))
        }
        AgentSessionLaunchUsecaseError::TerminalUnavailable
        | AgentSessionLaunchUsecaseError::TerminalSpawn(_) => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionTerminalUnavailable(kind))
        }
        AgentSessionLaunchUsecaseError::Store(_) => {
            AppError::new("Releash could not access saved AgentSession data. Try again.")
        }
        AgentSessionLaunchUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionCorrupt)
        }
    };
    result.with_status(kind)
}

pub(crate) fn lifecycle_error(error: AgentSessionLifecycleUsecaseError) -> AppError {
    let kind = error.connect_code();
    let result = match error {
        AgentSessionLifecycleUsecaseError::Workflow(error) => AppError::from_failure(error),
        AgentSessionLifecycleUsecaseError::NotFound => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionNotFound)
        }
        AgentSessionLifecycleUsecaseError::InvalidOperation => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionInvalidOperation)
        }
        AgentSessionLifecycleUsecaseError::Conflict(_) => provider_tui_coded_error(
            ProviderTuiCodedError::AgentSessionConflict(AgentSessionConflictOperation::Update),
        ),
        AgentSessionLifecycleUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionStorageUnavailable)
        }
        AgentSessionLifecycleUsecaseError::LaunchUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionLaunchUnavailable(kind))
        }
        AgentSessionLifecycleUsecaseError::TerminalUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionTerminalUnavailable(kind))
        }
        AgentSessionLifecycleUsecaseError::Store(_) => {
            AppError::new("Releash could not access saved AgentSession data. Try again.")
        }
        AgentSessionLifecycleUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionCorrupt)
        }
    };
    result.with_status(kind)
}

pub(crate) fn hook_health_error(error: ProviderHookHealthUsecaseError) -> AppError {
    let kind = error.connect_code();
    let result = match error {
        ProviderHookHealthUsecaseError::InvalidInput => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthInvalidRequest)
        }
        ProviderHookHealthUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthStorageUnavailable)
        }
        ProviderHookHealthUsecaseError::Conflict => {
            AppError::from_failure(crate::domain::workflow::WorkflowError::Conflict(
                "Provider Hook health changed. Refresh and try again.".into(),
            ))
        }
        ProviderHookHealthUsecaseError::Store(_) => {
            AppError::new("Releash could not load Provider Hook health. Try again.")
        }
        ProviderHookHealthUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthCorrupt)
        }
    };
    result.with_status(kind)
}

#[cfg(test)]
#[path = "provider_tui_test.rs"]
mod provider_tui_tests;
