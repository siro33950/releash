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
    AgentSessionLifecycleDto, AgentSessionLifecycleUsecase, AgentSessionLifecycleUsecaseError,
    AgentSessionListPageDto, AgentSessionListRequest, AgentSessionOpenOutcome,
    AgentSessionOriginFilter, AgentSessionProviderDto, AgentSessionReadUsecase,
    AgentSessionReadUsecaseError, ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
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
    let provider = parse_provider(&provider)?;
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
    let provider = parse_provider(&provider)?;
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

fn provider_availability_error(error: ProviderAvailabilityUsecaseError) -> AppError {
    match error {
        ProviderAvailabilityUsecaseError::InvalidInput => AppError::coded(
            "PROVIDER_AVAILABILITY_INVALID_EXECUTABLE",
            "Provider executable must be a non-empty command name or path",
        ),
        ProviderAvailabilityUsecaseError::ConfigUnavailable => AppError::coded(
            "PROVIDER_AVAILABILITY_CONFIG_UNAVAILABLE",
            "Provider executable Config is unavailable",
        ),
        ProviderAvailabilityUsecaseError::RefreshUnavailable => AppError::coded(
            "PROVIDER_AVAILABILITY_REFRESH_UNAVAILABLE",
            "Provider executable search environment refresh failed",
        ),
        ProviderAvailabilityUsecaseError::Corrupt => AppError::coded(
            "PROVIDER_AVAILABILITY_CORRUPT",
            "Provider availability registry is unavailable",
        ),
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
    let provider = parse_provider(&provider)?;
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
        .map_err(launch_error)
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
    let provider = parse_provider(&provider)?;
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
        .map_err(launch_error)?;
    Ok(match outcome {
        crate::usecase::agent_session::AgentSessionHistoryResumeOutcome::Open(session)
        | crate::usecase::agent_session::AgentSessionHistoryResumeOutcome::Paused(session) => {
            session.session().id().to_string()
        }
    })
}

fn parse_provider(value: &str) -> Result<ProviderKind, AppError> {
    match value {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(AppError::coded(
            "AGENT_SESSION_INVALID_PROVIDER",
            "Provider must be selected explicitly",
        )),
    }
}

#[tauri::command]
pub async fn list_agent_sessions(
    read: State<'_, Arc<AgentSessionReadUsecase>>,
    workspace_identity: String,
    lifecycle: Option<String>,
    origin: Option<String>,
    limit: Option<usize>,
    after_session_id: Option<String>,
) -> Result<AgentSessionListPageDto, AppError> {
    let lifecycle = lifecycle.as_deref().map(parse_lifecycle).transpose()?;
    let origin = origin.as_deref().map(parse_origin).transpose()?;
    read.list(AgentSessionListRequest {
        workspace: WorkspaceIdentity::new(workspace_identity),
        lifecycle,
        origin,
        limit: limit.unwrap_or(100),
        after_session_id,
    })
    .await
    .map_err(read_error)
}

fn parse_origin(value: &str) -> Result<AgentSessionOriginFilter, AppError> {
    match value {
        "standalone" => Ok(AgentSessionOriginFilter::Standalone),
        "workflow_node" => Ok(AgentSessionOriginFilter::WorkflowNode),
        _ => Err(AppError::coded(
            "AGENT_SESSION_INVALID_ORIGIN",
            "AgentSession origin is invalid",
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

fn parse_lifecycle(value: &str) -> Result<AgentSessionLifecycleDto, AppError> {
    match value {
        "open" => Ok(AgentSessionLifecycleDto::Open),
        "paused" => Ok(AgentSessionLifecycleDto::Paused),
        "archived" => Ok(AgentSessionLifecycleDto::Archived),
        _ => Err(AppError::coded(
            "AGENT_SESSION_INVALID_LIFECYCLE",
            "AgentSession lifecycle is invalid",
        )),
    }
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

fn launch_error(error: AgentSessionLaunchUsecaseError) -> AppError {
    match error {
        AgentSessionLaunchUsecaseError::ProviderUnavailable => AppError::coded(
            "AGENT_SESSION_PROVIDER_UNAVAILABLE",
            "Selected Provider is unavailable",
        ),
        AgentSessionLaunchUsecaseError::InvalidInput => AppError::coded(
            "AGENT_SESSION_INVALID_INPUT",
            "AgentSession launch input is invalid",
        ),
        AgentSessionLaunchUsecaseError::Conflict => AppError::coded(
            "AGENT_SESSION_CONFLICT",
            "AgentSession conflicts with current state",
        ),
        AgentSessionLaunchUsecaseError::StorageUnavailable => AppError::coded(
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "AgentSession persistence is unavailable",
        ),
        AgentSessionLaunchUsecaseError::LaunchUnavailable => AppError::coded(
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Provider launch preparation is unavailable",
        ),
        AgentSessionLaunchUsecaseError::TerminalUnavailable => AppError::coded(
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "AgentSession Terminal Surface is unavailable",
        ),
        AgentSessionLaunchUsecaseError::Corrupt => {
            AppError::coded("AGENT_SESSION_CORRUPT", "AgentSession state is corrupt")
        }
    }
}

fn lifecycle_error(error: AgentSessionLifecycleUsecaseError) -> AppError {
    match error {
        AgentSessionLifecycleUsecaseError::NotFound => {
            AppError::coded("AGENT_SESSION_NOT_FOUND", "AgentSession was not found")
        }
        AgentSessionLifecycleUsecaseError::InvalidOperation => AppError::coded(
            "AGENT_SESSION_INVALID_OPERATION",
            "AgentSession operation is not allowed in the current state",
        ),
        AgentSessionLifecycleUsecaseError::Conflict => AppError::coded(
            "AGENT_SESSION_CONFLICT",
            "AgentSession conflicts with current state",
        ),
        AgentSessionLifecycleUsecaseError::StorageUnavailable => AppError::coded(
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "AgentSession persistence is unavailable",
        ),
        AgentSessionLifecycleUsecaseError::LaunchUnavailable => AppError::coded(
            "AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Provider launch preparation is unavailable",
        ),
        AgentSessionLifecycleUsecaseError::TerminalUnavailable => AppError::coded(
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "AgentSession Terminal Surface is unavailable",
        ),
        AgentSessionLifecycleUsecaseError::Corrupt => {
            AppError::coded("AGENT_SESSION_CORRUPT", "AgentSession state is corrupt")
        }
    }
}

fn read_error(error: AgentSessionReadUsecaseError) -> AppError {
    match error {
        AgentSessionReadUsecaseError::InvalidRequest => AppError::coded(
            "AGENT_SESSION_INVALID_REQUEST",
            "AgentSession read request is invalid",
        ),
        AgentSessionReadUsecaseError::StorageUnavailable => AppError::coded(
            "AGENT_SESSION_STORAGE_UNAVAILABLE",
            "AgentSession persistence is unavailable",
        ),
        AgentSessionReadUsecaseError::TerminalUnavailable => AppError::coded(
            "AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "AgentSession Terminal Surface is unavailable",
        ),
        AgentSessionReadUsecaseError::Corrupt => {
            AppError::coded("AGENT_SESSION_CORRUPT", "AgentSession state is corrupt")
        }
    }
}

fn history_error(error: AgentSessionHistoryQueryError) -> AppError {
    match error {
        AgentSessionHistoryQueryError::InvalidRequest => AppError::coded(
            "AGENT_SESSION_HISTORY_INVALID_REQUEST",
            "AgentSession history request is invalid",
        ),
        AgentSessionHistoryQueryError::Unavailable => AppError::coded(
            "AGENT_SESSION_HISTORY_UNAVAILABLE",
            "AgentSession history is unavailable",
        ),
        AgentSessionHistoryQueryError::Corrupt => AppError::coded(
            "AGENT_SESSION_HISTORY_CORRUPT",
            "AgentSession history is corrupt",
        ),
    }
}

fn hook_health_error(error: ProviderHookHealthUsecaseError) -> AppError {
    match error {
        ProviderHookHealthUsecaseError::InvalidInput => AppError::coded(
            "PROVIDER_HOOK_HEALTH_INVALID_REQUEST",
            "Provider Hook health request is invalid",
        ),
        ProviderHookHealthUsecaseError::StorageUnavailable => AppError::coded(
            "PROVIDER_HOOK_HEALTH_STORAGE_UNAVAILABLE",
            "Provider Hook health persistence is unavailable",
        ),
        ProviderHookHealthUsecaseError::Corrupt => AppError::coded(
            "PROVIDER_HOOK_HEALTH_CORRUPT",
            "Provider Hook health state is corrupt",
        ),
    }
}

#[cfg(test)]
#[path = "provider_tui_test.rs"]
mod provider_tui_tests;
