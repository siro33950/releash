use std::sync::Arc;
use std::time::Instant;

use tauri::State;

use crate::adaptor::protocol::provider_agent_session::{
    ProviderAgentSessionArchiveResponse, ProviderAgentSessionOpenResponse,
    ProviderAvailabilitySnapshotResponse, ProviderHookHealthProviderResponse,
    ProviderHookHealthWarningResponse,
};
use crate::domain::agent_session::aggregates::AgentSessionArchiveOutcome;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::other::error::AppError;
use crate::usecase::agent_session::{
    ProviderAgentSessionHistoryPageDto, ProviderAgentSessionHistoryQueryError,
    ProviderAgentSessionHistoryReadUsecase, ProviderAgentSessionHistoryRequest,
    ProviderAgentSessionHistoryResumeRequest, ProviderAgentSessionItemDto,
    ProviderAgentSessionLaunchRequest, ProviderAgentSessionLaunchUsecase,
    ProviderAgentSessionLaunchUsecaseError, ProviderAgentSessionLifecycleDto,
    ProviderAgentSessionLifecycleUsecase, ProviderAgentSessionLifecycleUsecaseError,
    ProviderAgentSessionListPageDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionOpenOutcome, ProviderAgentSessionOriginFilter,
    ProviderAgentSessionProviderDto, ProviderAgentSessionReadUsecase,
    ProviderAgentSessionReadUsecaseError, ProviderAvailabilityUsecase,
    ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReadUsecase, ProviderHookHealthUsecaseError, ProviderHookHealthWarning,
};

#[tauri::command]
pub fn list_available_provider_agent_session_providers(
    availability: State<'_, Arc<ProviderAvailabilityUsecase>>,
) -> Result<Vec<ProviderAgentSessionProviderDto>, AppError> {
    availability
        .available_providers()
        .map(|providers| {
            providers
                .into_iter()
                .map(|provider| match provider {
                    ProviderKind::Claude => ProviderAgentSessionProviderDto::Claude,
                    ProviderKind::Codex => ProviderAgentSessionProviderDto::Codex,
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
pub async fn create_provider_agent_session(
    launch: State<'_, Arc<ProviderAgentSessionLaunchUsecase>>,
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
        .launch_standalone_idempotent(ProviderAgentSessionLaunchRequest {
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
pub async fn resume_provider_agent_session_history_candidate(
    launch: State<'_, Arc<ProviderAgentSessionLaunchUsecase>>,
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
        .resume_history(ProviderAgentSessionHistoryResumeRequest {
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
        crate::usecase::agent_session::ProviderAgentSessionHistoryResumeOutcome::Open(session)
        | crate::usecase::agent_session::ProviderAgentSessionHistoryResumeOutcome::Paused(
            session,
        ) => session.session().id().to_string(),
    })
}

fn parse_provider(value: &str) -> Result<ProviderKind, AppError> {
    match value {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        _ => Err(AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_PROVIDER",
            "Provider must be selected explicitly",
        )),
    }
}

#[tauri::command]
pub async fn list_provider_agent_sessions(
    read: State<'_, Arc<ProviderAgentSessionReadUsecase>>,
    workspace_identity: String,
    lifecycle: Option<String>,
    origin: Option<String>,
    limit: Option<usize>,
    after_session_id: Option<String>,
) -> Result<ProviderAgentSessionListPageDto, AppError> {
    let lifecycle = lifecycle.as_deref().map(parse_lifecycle).transpose()?;
    let origin = origin.as_deref().map(parse_origin).transpose()?;
    read.list(ProviderAgentSessionListRequest {
        workspace: WorkspaceIdentity::new(workspace_identity),
        lifecycle,
        origin,
        limit: limit.unwrap_or(100),
        after_session_id,
    })
    .await
    .map_err(read_error)
}

fn parse_origin(value: &str) -> Result<ProviderAgentSessionOriginFilter, AppError> {
    match value {
        "standalone" => Ok(ProviderAgentSessionOriginFilter::Standalone),
        "workflow_node" => Ok(ProviderAgentSessionOriginFilter::WorkflowNode),
        _ => Err(AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_ORIGIN",
            "Provider AgentSession origin is invalid",
        )),
    }
}

#[tauri::command]
pub async fn get_provider_agent_session(
    read: State<'_, Arc<ProviderAgentSessionReadUsecase>>,
    agent_session_id: String,
) -> Result<Option<ProviderAgentSessionItemDto>, AppError> {
    read.get(&agent_session_id).await.map_err(read_error)
}

#[tauri::command]
pub async fn open_provider_agent_session(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<ProviderAgentSessionOpenResponse, AppError> {
    lifecycle
        .open(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn resume_provider_agent_session(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<ProviderAgentSessionOpenResponse, AppError> {
    lifecycle
        .resume(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn restore_provider_agent_session(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<ProviderAgentSessionOpenResponse, AppError> {
    lifecycle
        .restore(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn archive_provider_agent_session(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<ProviderAgentSessionArchiveResponse, AppError> {
    lifecycle
        .archive(&agent_session_id, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn delete_provider_agent_session(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<(), AppError> {
    lifecycle
        .delete(&agent_session_id, &caller_request_id)
        .await
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn confirm_provider_agent_session_archive_delete(
    lifecycle: State<'_, Arc<ProviderAgentSessionLifecycleUsecase>>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<(), AppError> {
    lifecycle
        .confirm_archive_fallback_delete(&agent_session_id, &caller_request_id)
        .await
        .map_err(lifecycle_error)
}

#[tauri::command]
pub async fn list_provider_agent_session_history(
    query: State<'_, Arc<ProviderAgentSessionHistoryReadUsecase>>,
    worktree_path: String,
    limit: Option<usize>,
    after: Option<String>,
) -> Result<ProviderAgentSessionHistoryPageDto, AppError> {
    query
        .list(ProviderAgentSessionHistoryRequest {
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

fn parse_lifecycle(value: &str) -> Result<ProviderAgentSessionLifecycleDto, AppError> {
    match value {
        "open" => Ok(ProviderAgentSessionLifecycleDto::Open),
        "paused" => Ok(ProviderAgentSessionLifecycleDto::Paused),
        "archived" => Ok(ProviderAgentSessionLifecycleDto::Archived),
        _ => Err(AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_LIFECYCLE",
            "Provider AgentSession lifecycle is invalid",
        )),
    }
}

impl From<ProviderAgentSessionOpenOutcome> for ProviderAgentSessionOpenResponse {
    fn from(value: ProviderAgentSessionOpenOutcome) -> Self {
        match value {
            ProviderAgentSessionOpenOutcome::Attached => Self::Attached,
            ProviderAgentSessionOpenOutcome::Resumed => Self::Resumed,
            ProviderAgentSessionOpenOutcome::Restored => Self::Restored,
            ProviderAgentSessionOpenOutcome::Paused => Self::Paused,
            ProviderAgentSessionOpenOutcome::Indeterminate => Self::Indeterminate,
            ProviderAgentSessionOpenOutcome::GarbageCollected => Self::GarbageCollected,
        }
    }
}

impl From<AgentSessionArchiveOutcome> for ProviderAgentSessionArchiveResponse {
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

fn launch_error(error: ProviderAgentSessionLaunchUsecaseError) -> AppError {
    match error {
        ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_PROVIDER_UNAVAILABLE",
            "Selected Provider is unavailable",
        ),
        ProviderAgentSessionLaunchUsecaseError::InvalidInput => AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_INPUT",
            "Provider AgentSession launch input is invalid",
        ),
        ProviderAgentSessionLaunchUsecaseError::Conflict => AppError::coded(
            "PROVIDER_AGENT_SESSION_CONFLICT",
            "Provider AgentSession conflicts with current state",
        ),
        ProviderAgentSessionLaunchUsecaseError::StorageUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Provider AgentSession persistence is unavailable",
        ),
        ProviderAgentSessionLaunchUsecaseError::LaunchUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Provider launch preparation is unavailable",
        ),
        ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Provider AgentSession Terminal Surface is unavailable",
        ),
        ProviderAgentSessionLaunchUsecaseError::Corrupt => AppError::coded(
            "PROVIDER_AGENT_SESSION_CORRUPT",
            "Provider AgentSession state is corrupt",
        ),
    }
}

fn lifecycle_error(error: ProviderAgentSessionLifecycleUsecaseError) -> AppError {
    match error {
        ProviderAgentSessionLifecycleUsecaseError::NotFound => AppError::coded(
            "PROVIDER_AGENT_SESSION_NOT_FOUND",
            "Provider AgentSession was not found",
        ),
        ProviderAgentSessionLifecycleUsecaseError::InvalidOperation => AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_OPERATION",
            "Provider AgentSession operation is not allowed in the current state",
        ),
        ProviderAgentSessionLifecycleUsecaseError::Conflict => AppError::coded(
            "PROVIDER_AGENT_SESSION_CONFLICT",
            "Provider AgentSession conflicts with current state",
        ),
        ProviderAgentSessionLifecycleUsecaseError::StorageUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Provider AgentSession persistence is unavailable",
        ),
        ProviderAgentSessionLifecycleUsecaseError::LaunchUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_LAUNCH_UNAVAILABLE",
            "Provider launch preparation is unavailable",
        ),
        ProviderAgentSessionLifecycleUsecaseError::TerminalUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Provider AgentSession Terminal Surface is unavailable",
        ),
        ProviderAgentSessionLifecycleUsecaseError::Corrupt => AppError::coded(
            "PROVIDER_AGENT_SESSION_CORRUPT",
            "Provider AgentSession state is corrupt",
        ),
    }
}

fn read_error(error: ProviderAgentSessionReadUsecaseError) -> AppError {
    match error {
        ProviderAgentSessionReadUsecaseError::InvalidRequest => AppError::coded(
            "PROVIDER_AGENT_SESSION_INVALID_REQUEST",
            "Provider AgentSession read request is invalid",
        ),
        ProviderAgentSessionReadUsecaseError::StorageUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_STORAGE_UNAVAILABLE",
            "Provider AgentSession persistence is unavailable",
        ),
        ProviderAgentSessionReadUsecaseError::TerminalUnavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_TERMINAL_UNAVAILABLE",
            "Provider AgentSession Terminal Surface is unavailable",
        ),
        ProviderAgentSessionReadUsecaseError::Corrupt => AppError::coded(
            "PROVIDER_AGENT_SESSION_CORRUPT",
            "Provider AgentSession state is corrupt",
        ),
    }
}

fn history_error(error: ProviderAgentSessionHistoryQueryError) -> AppError {
    match error {
        ProviderAgentSessionHistoryQueryError::InvalidRequest => AppError::coded(
            "PROVIDER_AGENT_SESSION_HISTORY_INVALID_REQUEST",
            "Provider AgentSession history request is invalid",
        ),
        ProviderAgentSessionHistoryQueryError::Unavailable => AppError::coded(
            "PROVIDER_AGENT_SESSION_HISTORY_UNAVAILABLE",
            "Provider AgentSession history is unavailable",
        ),
        ProviderAgentSessionHistoryQueryError::Corrupt => AppError::coded(
            "PROVIDER_AGENT_SESSION_HISTORY_CORRUPT",
            "Provider AgentSession history is corrupt",
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
