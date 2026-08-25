use std::sync::Arc;
use std::time::Instant;

use tauri::State;

use crate::adaptor::protocol::agent_session::{
    AgentSessionArchiveResponse, AgentSessionOpenResponse, ProviderAvailabilitySnapshotResponse,
    ProviderHookHealthProviderResponse, ProviderHookHealthWarningResponse,
};
use crate::domain::agent_session::aggregates::AgentSessionArchiveOutcome;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::other::error::AppError;
use crate::usecase::agent_session::{
    AgentSessionHistoryPageDto, AgentSessionHistoryQueryError, AgentSessionHistoryReadUsecase,
    AgentSessionHistoryRequest, AgentSessionHistoryResumeRequest, AgentSessionItemDto,
    AgentSessionLaunchRequest, AgentSessionLaunchUsecase, AgentSessionLaunchUsecaseError,
    AgentSessionLifecycleUsecase, AgentSessionLifecycleUsecaseError, AgentSessionOpenOutcome,
    AgentSessionProviderDto, AgentSessionReadUsecase, AgentSessionReadUsecaseError,
    ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReadUsecase, ProviderHookHealthUsecaseError, ProviderHookHealthWarning,
};

#[tauri::command]
pub fn list_available_agent_session_providers(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
) -> Result<Vec<AgentSessionProviderDto>, AppError> {
    availability
        .available_providers()
        .map(|providers| {
            providers
                .into_iter()
                .map(|provider| match provider {
                    ProviderKind::Claude => AgentSessionProviderDto::Claude,
                    ProviderKind::Codex => AgentSessionProviderDto::Codex,
                })
                .collect()
        })
        .map_err(provider_availability_error)
}

#[tauri::command]
pub fn get_provider_availability(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    availability
        .snapshot()
        .map(Into::into)
        .map_err(provider_availability_error)
}

#[tauri::command]
pub async fn refresh_provider_availability(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let availability = Arc::clone(availability.inner());
    run_provider_availability_blocking(move || availability.refresh())
        .await
        .map(Into::into)
}

#[tauri::command]
pub async fn update_provider_executable(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
    provider: String,
    executable: String,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability.inner());
    run_provider_availability_blocking(move || {
        availability.update_configured_executable(provider, &executable)
    })
    .await
    .map(Into::into)
}

#[tauri::command]
pub async fn reset_provider_executable(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
    provider: String,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability.inner());
    run_provider_availability_blocking(move || availability.reset_configured_executable(provider))
        .await
        .map(Into::into)
}

async fn run_provider_availability_blocking<T, F>(operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ProviderAvailabilityUsecaseError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| provider_availability_error(ProviderAvailabilityUsecaseError::Corrupt))?
        .map_err(provider_availability_error)
}

#[derive(Clone, Copy)]
enum ProviderParseOperation {
    ConfigureProvider,
    Start,
    ResumeHistory,
}

#[derive(Clone, Copy)]
enum AgentSessionLaunchOperation {
    Start,
    ResumeHistory,
}

#[derive(Clone, Copy)]
enum AgentSessionConflictOperation {
    Start,
    ResumeHistory,
    Update,
}

enum ProviderTuiCodedError {
    ProviderAvailabilityInvalidExecutable,
    ProviderAvailabilityConfigUnavailable,
    ProviderAvailabilityRefreshUnavailable,
    ProviderAvailabilityCorrupt,
    AgentSessionInvalidProvider(ProviderParseOperation),
    AgentSessionProviderUnavailable,
    AgentSessionInvalidInput(AgentSessionLaunchOperation),
    AgentSessionConflict(AgentSessionConflictOperation),
    AgentSessionStorageUnavailable,
    AgentSessionLaunchUnavailable,
    AgentSessionTerminalUnavailable,
    AgentSessionCorrupt,
    AgentSessionNotFound,
    AgentSessionInvalidOperation,
    AgentSessionInvalidRequest,
    AgentSessionHistoryInvalidRequest,
    AgentSessionHistoryUnavailable,
    AgentSessionHistoryCorrupt,
    ProviderHookHealthInvalidRequest,
    ProviderHookHealthStorageUnavailable,
    ProviderHookHealthCorrupt,
}

fn provider_tui_coded_error(error: ProviderTuiCodedError) -> AppError {
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
        ProviderTuiCodedError::AgentSessionLaunchUnavailable => (
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Releash could not complete the Provider operation for this AgentSession. Try again.",
        ),
        ProviderTuiCodedError::AgentSessionTerminalUnavailable => (
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
        ProviderTuiCodedError::AgentSessionInvalidRequest => (
            "AGENT_SESSION_INVALID_REQUEST",
            "Releash could not load the AgentSession because the request is invalid.",
        ),
        ProviderTuiCodedError::AgentSessionHistoryInvalidRequest => (
            "AGENT_SESSION_HISTORY_INVALID_REQUEST",
            "Releash could not load AgentSession history because the request is invalid.",
        ),
        ProviderTuiCodedError::AgentSessionHistoryUnavailable => (
            "AGENT_SESSION_HISTORY_UNAVAILABLE",
            "Releash could not load AgentSession history. Try again.",
        ),
        ProviderTuiCodedError::AgentSessionHistoryCorrupt => (
            "AGENT_SESSION_HISTORY_CORRUPT",
            "Releash could not load AgentSession history because its saved data is invalid.",
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
    AppError::coded(code, message)
}

fn provider_availability_error(error: ProviderAvailabilityUsecaseError) -> AppError {
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

#[tauri::command]
pub async fn create_agent_session(
    launch: State<'_, Arc<AgentSessionLaunchUsecase>>,
    workspace_identity: String,
    worktree_path: String,
    provider: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<String, AppError> {
    let command_ingress = Instant::now();
    let provider = parse_provider(&provider, ProviderParseOperation::Start)?;
    crate::other::telemetry::record_terminal_launch(
        crate::other::telemetry::TerminalLaunch::CommandIngress,
        command_ingress.elapsed(),
    );
    Arc::clone(launch.inner())
        .launch_standalone_idempotent(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new(workspace_identity),
            worktree_path,
            provider,
            rows,
            cols,
            caller_request_id,
        })
        .await
        .map_err(|error| launch_error(error, AgentSessionLaunchOperation::Start))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn resume_agent_session_history_candidate(
    launch: State<'_, Arc<AgentSessionLaunchUsecase>>,
    workspace_identity: String,
    worktree_path: String,
    provider: String,
    provider_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<String, AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ResumeHistory)?;
    let outcome = launch
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new(workspace_identity),
            worktree_path,
            provider,
            provider_session_id,
            rows,
            cols,
            caller_request_id,
        })
        .await
        .map_err(|error| launch_error(error, AgentSessionLaunchOperation::ResumeHistory))?;
    Ok(match outcome {
        crate::usecase::agent_session::AgentSessionHistoryResumeOutcome::Open(session)
        | crate::usecase::agent_session::AgentSessionHistoryResumeOutcome::Paused(session) => {
            session.session().id().to_string()
        }
    })
}

fn parse_provider(
    value: &str,
    operation: ProviderParseOperation,
) -> Result<ProviderKind, AppError> {
    match value {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(provider_tui_coded_error(
            ProviderTuiCodedError::AgentSessionInvalidProvider(operation),
        )),
    }
}

#[tauri::command]
pub async fn get_agent_session(
    read: State<'_, Arc<AgentSessionReadUsecase>>,
    agent_session_id: String,
) -> Result<Option<AgentSessionItemDto>, AppError> {
    read.get(&agent_session_id).await.map_err(read_error)
}

#[tauri::command]
pub async fn open_agent_session(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<AgentSessionOpenResponse, AppError> {
    lifecycle
        .open(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn resume_agent_session(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<AgentSessionOpenResponse, AppError> {
    lifecycle
        .resume(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn restore_agent_session(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<AgentSessionOpenResponse, AppError> {
    lifecycle
        .restore(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn archive_agent_session(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<AgentSessionArchiveResponse, AppError> {
    lifecycle
        .archive(&agent_session_id, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn delete_agent_session(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<(), AppError> {
    lifecycle
        .delete(&agent_session_id, &caller_request_id)
        .await
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn confirm_agent_session_archive_delete(
    lifecycle: State<'_, Arc<AgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<(), AppError> {
    lifecycle
        .confirm_archive_fallback_delete(&agent_session_id, &caller_request_id)
        .await
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn list_agent_session_history(
    query: State<'_, Arc<AgentSessionHistoryReadUsecase>>,
    worktree_path: String,
    limit: Option<usize>,
    after: Option<String>,
) -> Result<AgentSessionHistoryPageDto, AppError> {
    query
        .list(AgentSessionHistoryRequest {
            worktree_path,
            limit: limit.unwrap_or(100),
            after,
        })
        .await
        .map_err(history_error)
}

#[tauri::command]
pub async fn list_provider_hook_health_warnings(
    query: State<'_, Arc<ProviderHookHealthReadUsecase>>,
) -> Result<Vec<ProviderHookHealthWarningResponse>, AppError> {
    query
        .warnings()
        .await
        .map(|warnings| warnings.into_iter().map(Into::into).collect())
        .map_err(hook_health_error)
}

impl From<AgentSessionOpenOutcome> for AgentSessionOpenResponse {
    fn from(value: AgentSessionOpenOutcome) -> Self {
        match value {
            AgentSessionOpenOutcome::Attached => Self::Attached,
            AgentSessionOpenOutcome::Resumed => Self::Resumed,
            AgentSessionOpenOutcome::Restored => Self::Restored,
            AgentSessionOpenOutcome::Paused => Self::Paused,
            AgentSessionOpenOutcome::Indeterminate => Self::Indeterminate,
            AgentSessionOpenOutcome::GarbageCollected => Self::GarbageCollected,
        }
    }
}

impl From<AgentSessionArchiveOutcome> for AgentSessionArchiveResponse {
    fn from(value: AgentSessionArchiveOutcome) -> Self {
        match value {
            AgentSessionArchiveOutcome::Archived => Self::Archived,
            AgentSessionArchiveOutcome::AlreadyArchived => Self::AlreadyArchived,
            AgentSessionArchiveOutcome::DeleteConfirmationRequired => {
                Self::DeleteConfirmationRequired
            }
        }
    }
}

impl From<ProviderHookHealthWarning> for ProviderHookHealthWarningResponse {
    fn from(value: ProviderHookHealthWarning) -> Self {
        Self {
            provider: match value.provider {
                crate::domain::provider_lifecycle::ProviderKind::Claude => {
                    ProviderHookHealthProviderResponse::Claude
                }
                crate::domain::provider_lifecycle::ProviderKind::Codex => {
                    ProviderHookHealthProviderResponse::Codex
                }
            },
            launch_id: value.launch_id,
            reason: match value.reason {
                crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded => "session_start_deadline_exceeded",
                crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed => "codex_hook_delivery_unconfirmed",
                crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::ProviderHookConfigurationRejected => "provider_hook_configuration_rejected",
                crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::LocalApiUnavailable => "local_api_unavailable",
            }
            .to_string(),
        }
    }
}

fn launch_error(
    error: AgentSessionLaunchUsecaseError,
    operation: AgentSessionLaunchOperation,
) -> AppError {
    match error {
        AgentSessionLaunchUsecaseError::ProviderUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionProviderUnavailable)
        }
        AgentSessionLaunchUsecaseError::InvalidInput => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionInvalidInput(operation))
        }
        AgentSessionLaunchUsecaseError::Conflict => {
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
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionLaunchUnavailable)
        }
        AgentSessionLaunchUsecaseError::TerminalUnavailable
        | AgentSessionLaunchUsecaseError::TerminalSpawn(_) => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionTerminalUnavailable)
        }
        AgentSessionLaunchUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionCorrupt)
        }
    }
}

fn lifecycle_error(error: AgentSessionLifecycleUsecaseError) -> AppError {
    match error {
        AgentSessionLifecycleUsecaseError::NotFound => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionNotFound)
        }
        AgentSessionLifecycleUsecaseError::InvalidOperation => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionInvalidOperation)
        }
        AgentSessionLifecycleUsecaseError::Conflict => provider_tui_coded_error(
            ProviderTuiCodedError::AgentSessionConflict(AgentSessionConflictOperation::Update),
        ),
        AgentSessionLifecycleUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionStorageUnavailable)
        }
        AgentSessionLifecycleUsecaseError::LaunchUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionLaunchUnavailable)
        }
        AgentSessionLifecycleUsecaseError::TerminalUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionTerminalUnavailable)
        }
        AgentSessionLifecycleUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionCorrupt)
        }
    }
}

fn read_error(error: AgentSessionReadUsecaseError) -> AppError {
    match error {
        AgentSessionReadUsecaseError::InvalidRequest => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionInvalidRequest)
        }
        AgentSessionReadUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionStorageUnavailable)
        }
        AgentSessionReadUsecaseError::TerminalUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionTerminalUnavailable)
        }
        AgentSessionReadUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionCorrupt)
        }
    }
}

fn history_error(error: AgentSessionHistoryQueryError) -> AppError {
    match error {
        AgentSessionHistoryQueryError::InvalidRequest => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionHistoryInvalidRequest)
        }
        AgentSessionHistoryQueryError::Unavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionHistoryUnavailable)
        }
        AgentSessionHistoryQueryError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::AgentSessionHistoryCorrupt)
        }
    }
}

fn hook_health_error(error: ProviderHookHealthUsecaseError) -> AppError {
    match error {
        ProviderHookHealthUsecaseError::InvalidInput => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthInvalidRequest)
        }
        ProviderHookHealthUsecaseError::StorageUnavailable => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthStorageUnavailable)
        }
        ProviderHookHealthUsecaseError::Corrupt => {
            provider_tui_coded_error(ProviderTuiCodedError::ProviderHookHealthCorrupt)
        }
    }
}

#[cfg(test)]
#[path = "provider_tui_test.rs"]
mod provider_tui_tests;
