use crate::adaptor::presenter::provider_tui::{
    hook_health_error, launch_error, lifecycle_error, provider_availability_error,
    provider_tui_coded_error, AgentSessionLaunchOperation, ProviderParseOperation,
    ProviderTuiCodedError,
};
use std::sync::Arc;

use crate::adaptor::presenter::error::AppError;
use crate::adaptor::protocol::agent_session::{
    AgentSessionArchiveResponse, AgentSessionOpenResponse, ProviderAvailabilitySnapshotResponse,
    ProviderHookHealthProviderResponse, ProviderHookHealthWarningResponse,
};
use crate::domain::agent_session::aggregates::AgentSessionArchiveOutcome;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionHistoryResumeRequest, AgentSessionLaunchRequest, AgentSessionLaunchUsecase,
    AgentSessionLifecycleUsecase, AgentSessionOpenOutcome, ProviderAvailabilityUsecase,
    ProviderAvailabilityUsecaseError,
};
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthReadUsecase, ProviderHookHealthWarning,
};

pub(crate) fn get_provider_availability_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    availability
        .snapshot()
        .map(Into::into)
        .map_err(provider_availability_error)
}

pub(crate) async fn refresh_provider_availability_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || availability.refresh())
        .await
        .map(Into::into)
}

pub(crate) async fn update_provider_executable_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
    provider: String,
    executable: String,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || {
        availability.update_configured_executable(provider, &executable)
    })
    .await
    .map(Into::into)
}

pub(crate) async fn reset_provider_executable_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
    provider: String,
) -> Result<ProviderAvailabilitySnapshotResponse, AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || availability.reset_configured_executable(provider))
        .await
        .map(Into::into)
}

async fn run_provider_availability_blocking<T, F>(operation: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ProviderAvailabilityUsecaseError> + Send + 'static,
{
    crate::common::operation_context::spawn_blocking(operation)
        .await
        .map_err(|_| provider_availability_error(ProviderAvailabilityUsecaseError::Corrupt))?
        .map_err(provider_availability_error)
}

pub(crate) async fn create_agent_session_shared(
    launch: &Arc<AgentSessionLaunchUsecase>,
    workspace_identity: String,
    worktree_path: String,
    provider: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<String, AppError> {
    let provider = crate::common::telemetry::observe_result(
        || parse_provider(&provider, ProviderParseOperation::Start),
        |result, elapsed| {
            if result.is_ok() {
                crate::infrastructure::telemetry::metrics::record_terminal_launch(
                    crate::infrastructure::telemetry::metrics::TerminalLaunch::CommandIngress,
                    elapsed,
                );
            }
        },
    )?;
    Arc::clone(launch)
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

pub(crate) async fn resume_agent_session_history_candidate_shared(
    launch: &Arc<AgentSessionLaunchUsecase>,
    args: crate::adaptor::controller::api::protocol::client::ResumeAgentSessionHistoryCandidateRequest,
) -> Result<String, crate::adaptor::controller::api::protocol::client::CommandFailure> {
    use crate::adaptor::controller::client::{convert, required};
    let provider = parse_provider(
        &required(args.provider, "provider")?,
        ProviderParseOperation::ResumeHistory,
    )?;
    let outcome = launch
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new(required(
                args.workspace_identity,
                "workspaceIdentity",
            )?),
            worktree_path: required(args.worktree_path, "worktreePath")?,
            provider,
            provider_session_id: required(args.provider_session_id, "providerSessionId")?,
            rows: convert(required(args.rows, "rows")?)?,
            cols: convert(required(args.cols, "cols")?)?,
            caller_request_id: required(args.caller_request_id, "callerRequestId")?,
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

pub(crate) async fn open_agent_session_shared(
    lifecycle: &Arc<AgentSessionLifecycleUsecase>,
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

pub(crate) async fn restore_agent_session_shared(
    lifecycle: &Arc<AgentSessionLifecycleUsecase>,
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

pub(crate) async fn archive_agent_session_shared(
    lifecycle: &Arc<AgentSessionLifecycleUsecase>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<AgentSessionArchiveResponse, AppError> {
    lifecycle
        .archive(&agent_session_id, &caller_request_id)
        .await
        .map(Into::into)
        .map_err(lifecycle_error)
}

pub(crate) async fn delete_agent_session_shared(
    lifecycle: &Arc<AgentSessionLifecycleUsecase>,
    agent_session_id: String,
    caller_request_id: String,
) -> Result<(), AppError> {
    lifecycle
        .delete(&agent_session_id, &caller_request_id)
        .await
        .map_err(lifecycle_error)
}

pub(crate) async fn list_provider_hook_health_warnings_shared(
    query: &Arc<ProviderHookHealthReadUsecase>,
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

#[cfg(test)]
#[path = "provider_tui_test.rs"]
mod provider_tui_tests;
