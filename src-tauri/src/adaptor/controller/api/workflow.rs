use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::adaptor::presenter::workflow::workflow_execution_to_view;
use crate::adaptor::presenter::workflow_api::{
    DiagnosticReportResponse, WorkflowEventResponse, WorkflowExecutionSummaryResponse,
    WorkflowSummaryResponse,
};
use crate::adaptor::presenter::workflow_wire::WorkflowExecutionView;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatusFilter, WorkflowError, WorkflowPageRequest,
};
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, RetryNodeCommand, StartExecutionCommand,
    SubmitOutputArtifact, SubmitOutputCommand,
};
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;

use super::error::ApiError;
use super::protocol::{
    ApproveNodeRequest, RetryNodeRequest, StartExecutionRequest, SubmitOutputRequest,
    ValidateArtifactRequest,
};
use super::LocalApiState;
use crate::adaptor::presenter::api_response::{
    GetArtifactResponse, MutationResponse, StartExecutionResponse, ValidateArtifactResponse,
};

const DEFAULT_PAGE_LIMIT: u64 = 100;
const MAX_PAGE_LIMIT: u64 = 200;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionListQuery {
    #[serde(default)]
    worktree: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsQuery {
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionLogQuery {
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,
}

pub(super) fn router() -> Router<LocalApiState> {
    Router::new()
        .route("/v1/workflows", get(list_workflows))
        .route(
            "/v1/workflow/executions",
            get(list_executions).post(start_execution),
        )
        .route("/v1/workflow/executions/{execution_id}", get(get_execution))
        .route(
            "/v1/workflow/executions/{execution_id}/log",
            get(get_execution_log),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/approve",
            post(approve_node),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/abort",
            post(abort_execution),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/retry",
            post(retry_node),
        )
        .route(
            "/v1/workflow/node-executions/{node_execution_id}/resume",
            post(resume_session_node),
        )
        .route(
            "/v1/workflow/node-executions/{node_execution_id}/submit",
            post(submit_output),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/artifacts:validate",
            post(validate_artifact),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/artifacts/{node}",
            get(get_artifact),
        )
        .route("/v1/workflow/diagnostics", get(diagnose_workflows))
}

async fn list_workflows(
    State(state): State<LocalApiState>,
) -> Result<Json<Vec<WorkflowSummaryResponse>>, ApiError> {
    let workflow = state.workflow;
    let summaries = (workflow.list_workflow_summaries().await).map_err(ApiError::from)?;
    Ok(Json(summaries.into_iter().map(Into::into).collect()))
}

async fn diagnose_workflows(
    State(state): State<LocalApiState>,
    query: Result<Query<DiagnosticsQuery>, QueryRejection>,
) -> Result<Json<DiagnosticReportResponse>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let target = WorkflowDiagnosticsTarget::from_optional_directory(query.dir)?;
    let workflow = state.workflow;
    let report = blocking(move || workflow.diagnose_all(target)).await?;
    Ok(Json(report.into()))
}

async fn list_executions(
    State(state): State<LocalApiState>,
    query: Result<Query<ExecutionListQuery>, QueryRejection>,
) -> Result<Json<Vec<WorkflowExecutionSummaryResponse>>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let status = parse_status_filter(query.status.as_deref())?;
    let worktree = query.worktree;
    let page = parse_page(query.limit, query.offset)?;
    let workflow = state.workflow;
    let executions = (workflow
        .list_executions_filtered(status, worktree.as_deref(), page)
        .await)
        .map_err(ApiError::from)?;
    Ok(Json(executions.into_iter().map(Into::into).collect()))
}

async fn start_execution(
    State(state): State<LocalApiState>,
    payload: Result<Json<StartExecutionRequest>, JsonRejection>,
) -> Result<Json<StartExecutionResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let created_from = parse_execution_origin(payload.created_from.as_deref())?;
    let execution_id = state
        .runtime
        .start_execution(StartExecutionCommand {
            workflow_name: payload.workflow_name,
            worktree_path: payload.worktree_path,
            request: Some(payload.request),
            created_from,
        })
        .await?;
    Ok(Json(execution_id.into()))
}

async fn get_execution(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<WorkflowExecutionView>, ApiError> {
    let workflow = state.workflow;
    let lookup_id = execution_id.clone();
    let execution = (workflow.get_execution_state(&lookup_id).await)
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(format!("workflow execution '{execution_id}' was not found"))
        })?;
    Ok(Json(workflow_execution_to_view(execution)))
}

async fn get_execution_log(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
    query: Result<Query<ExecutionLogQuery>, QueryRejection>,
) -> Result<Json<Vec<WorkflowEventResponse>>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let page = parse_page(query.limit, query.offset)?;
    let workflow = state.workflow;
    let events =
        (workflow.get_execution_log_page(&execution_id, page).await).map_err(ApiError::from)?;
    Ok(Json(events.into_iter().map(Into::into).collect()))
}

async fn approve_node(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
    payload: Result<Json<ApproveNodeRequest>, JsonRejection>,
) -> Result<Json<MutationResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    state
        .runtime
        .resolve_approval(ApprovalCommand {
            execution_id,
            node_name: payload.node,
            node_execution_id: payload.node_execution_id,
            comment: payload.comment,
        })
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn abort_execution(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    state
        .runtime
        .abort_execution(AbortExecutionCommand {
            execution_id,
            expected_node_name: None,
        })
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn resume_session_node(
    State(state): State<LocalApiState>,
    Path(node_execution_id): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    state
        .runtime
        .resume_session_node_by_id(node_execution_id)
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn retry_node(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
    payload: Result<Json<RetryNodeRequest>, JsonRejection>,
) -> Result<Json<MutationResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    state
        .runtime
        .retry_node(RetryNodeCommand {
            execution_id,
            node_execution_id: payload.node_execution_id,
        })
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn submit_output(
    State(state): State<LocalApiState>,
    Path(node_execution_id): Path<String>,
    payload: Result<Json<SubmitOutputRequest>, JsonRejection>,
) -> Result<Json<MutationResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    state
        .runtime
        .submit_output(SubmitOutputCommand {
            node_execution_id,
            artifact: payload.artifact.map(|artifact| SubmitOutputArtifact {
                contract: artifact.contract,
                value: artifact.value,
            }),
        })
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn validate_artifact(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
    payload: Result<Json<ValidateArtifactRequest>, JsonRejection>,
) -> Result<Json<ValidateArtifactResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let workflow = state.workflow;
    let validation = ({
        workflow
            .validate_output_for_contract(
                &execution_id,
                &payload.node,
                &payload.contract,
                payload.value,
            )
            .await
    })
    .map_err(ApiError::from)?;
    Ok(Json(validation.into()))
}

async fn get_artifact(
    State(state): State<LocalApiState>,
    Path((execution_id, node)): Path<(String, String)>,
) -> Result<Json<GetArtifactResponse>, ApiError> {
    let workflow = state.workflow;
    let output = (workflow.get_output(&execution_id, &node).await).map_err(ApiError::from)?;
    Ok(Json(output.into()))
}

fn parse_status_filter(value: Option<&str>) -> Result<Option<ExecutionStatusFilter>, ApiError> {
    ExecutionStatusFilter::from_public_filter(value).map_err(|_| {
        ApiError::invalid_request(format!(
            "invalid status filter: {}",
            value.unwrap_or_default()
        ))
    })
}

fn parse_execution_origin(value: Option<&str>) -> Result<ExecutionOrigin, ApiError> {
    let Some(value) = value else {
        return Ok(ExecutionOrigin::Api);
    };
    match ExecutionOrigin::from_public_value(value) {
        Ok(origin @ (ExecutionOrigin::Api | ExecutionOrigin::Cli | ExecutionOrigin::Agent)) => {
            Ok(origin)
        }
        Ok(ExecutionOrigin::DesktopUi) | Err(_) => Err(ApiError::invalid_request(format!(
            "invalid created_from value: {value}"
        ))),
    }
}

fn parse_page(limit: Option<u64>, offset: Option<u64>) -> Result<WorkflowPageRequest, ApiError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(ApiError::invalid_request(format!(
            "limit must be between 1 and {MAX_PAGE_LIMIT}"
        )));
    }
    let offset = usize::try_from(offset.unwrap_or_default())
        .map_err(|_| ApiError::invalid_request("offset is too large"))?;
    Ok(WorkflowPageRequest::new(offset, limit as usize))
}

async fn blocking<T>(
    task: impl FnOnce() -> Result<T, WorkflowError> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    crate::common::operation_context::spawn_blocking(task)
        .await
        .map_err(|error| ApiError::from(crate::domain::failure::TechnicalFailure::from(error)))?
        .map_err(ApiError::from)
}

#[cfg(test)]
#[path = "workflow_test.rs"]
mod workflow_tests;
