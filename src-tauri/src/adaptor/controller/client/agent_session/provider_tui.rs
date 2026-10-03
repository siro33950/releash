use crate::adaptor::presenter::provider_tui::{
    launch_error, lifecycle_error, provider_availability_error, provider_tui_coded_error,
    AgentSessionLaunchOperation, ProviderParseOperation, ProviderTuiCodedError,
};
use std::sync::Arc;

use crate::adaptor::presenter::agent_session::AgentSessionArchiveResponse;
use crate::adaptor::presenter::error::AppError;
use crate::domain::agent_session::aggregates::AgentSessionArchiveOutcome;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionHistoryResumeRequest, AgentSessionLaunchRequest, AgentSessionLaunchUsecase,
    AgentSessionLifecycleUsecase, ProviderAvailabilityUsecase, ProviderAvailabilityUsecaseError,
};

pub(crate) async fn refresh_provider_availability_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
) -> Result<(), AppError> {
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || availability.refresh()).await
}

pub(crate) async fn update_provider_executable_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
    provider: String,
    executable: String,
) -> Result<(), AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || {
        availability.update_configured_executable(provider, &executable)
    })
    .await
}

pub(crate) async fn reset_provider_executable_shared(
    availability: &Arc<ProviderAvailabilityUsecase>,
    provider: String,
) -> Result<(), AppError> {
    let provider = parse_provider(&provider, ProviderParseOperation::ConfigureProvider)?;
    let availability = Arc::clone(availability);
    run_provider_availability_blocking(move || availability.reset_configured_executable(provider))
        .await
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
    args: crate::adaptor::presenter::client::CreateAgentSessionRequest,
) -> Result<
    crate::usecase::workspace_tree::SessionNodeSelectionDto,
    crate::adaptor::presenter::client::CommandFailure,
> {
    use crate::adaptor::controller::client::{convert, required};
    let provider = required(args.provider, "provider")?;
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
        .launch_standalone_selection(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new(required(
                args.workspace_identity,
                "workspaceIdentity",
            )?),
            worktree_path: required(args.worktree_path, "worktreePath")?,
            provider,
            rows: convert(required(args.rows, "rows")?)?,
            cols: convert(required(args.cols, "cols")?)?,
            caller_request_id: required(args.caller_request_id, "callerRequestId")?,
        })
        .await
        .map_err(|error| launch_error(error, AgentSessionLaunchOperation::Start).into())
}

pub(crate) async fn resume_agent_session_history_candidate_shared(
    launch: &Arc<AgentSessionLaunchUsecase>,
    args: crate::adaptor::presenter::client::ResumeAgentSessionHistoryCandidateRequest,
) -> Result<
    crate::usecase::workspace_tree::SessionNodeSelectionDto,
    crate::adaptor::presenter::client::CommandFailure,
> {
    use crate::adaptor::controller::client::{convert, required};
    let provider = parse_provider(
        &required(args.provider, "provider")?,
        ProviderParseOperation::ResumeHistory,
    )?;
    launch
        .resume_history_selection(AgentSessionHistoryResumeRequest {
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
        .map_err(|error| launch_error(error, AgentSessionLaunchOperation::ResumeHistory).into())
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
) -> Result<(), AppError> {
    lifecycle
        .open(&agent_session_id, rows, cols, &caller_request_id)
        .await
        .map(|_| ())
        .map_err(lifecycle_error)
}

pub(crate) async fn restore_agent_session_shared(
    lifecycle: &Arc<AgentSessionLifecycleUsecase>,
    agent_session_id: String,
    rows: u16,
    cols: u16,
    caller_request_id: String,
) -> Result<crate::usecase::workspace_tree::SessionNodeSelectionDto, AppError> {
    lifecycle
        .restore_selection(&agent_session_id, rows, cols, &caller_request_id)
        .await
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

impl From<AgentSessionArchiveOutcome> for AgentSessionArchiveResponse {
    fn from(value: AgentSessionArchiveOutcome) -> Self {
        match value {
            AgentSessionArchiveOutcome::Archived => Self::Archived,
            AgentSessionArchiveOutcome::AlreadyArchived => Self::AlreadyArchived,
        }
    }
}

#[cfg(test)]
#[path = "provider_tui_test.rs"]
mod provider_tui_tests;
