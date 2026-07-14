use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::adaptor::presenter::workflow::workflow_execution_to_view;
use crate::adaptor::protocol::workflow::WorkflowExecutionView;
use crate::domain::workflow::{ExecutionOrigin, ExecutionStatusFilter, WorkflowError};
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, StartExecutionCommand, SubmitOutputCommand,
};
use crate::usecase::workflow::dto::{WorkflowExecutionSummaryDto, WorkflowSummaryDto};

use super::error::ApiError;
use super::protocol::{
    ApproveNodeRequest, GetArtifactResponse, MutationResponse, StartExecutionRequest,
    StartExecutionResponse, SubmitArtifactRequest, ValidateArtifactRequest,
    ValidateArtifactResponse,
};
use super::LocalApiState;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionListQuery {
    #[serde(default)]
    worktree: Option<String>,
    #[serde(default)]
    status: Option<String>,
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
            "/v1/workflow/executions/{execution_id}/artifacts",
            post(submit_artifact),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/artifacts:validate",
            post(validate_artifact),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/artifacts/{node}",
            get(get_artifact),
        )
}

async fn list_workflows(
    State(state): State<LocalApiState>,
) -> Result<Json<Vec<WorkflowSummaryDto>>, ApiError> {
    let workflow = state.workflow;
    let summaries = blocking(move || workflow.list_workflow_summaries()).await?;
    Ok(Json(summaries))
}

async fn list_executions(
    State(state): State<LocalApiState>,
    query: Result<Query<ExecutionListQuery>, QueryRejection>,
) -> Result<Json<Vec<WorkflowExecutionSummaryDto>>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let status = parse_status_filter(query.status.as_deref())?;
    let worktree = query.worktree;
    let workflow = state.workflow;
    let executions =
        blocking(move || workflow.list_executions_filtered(status, worktree.as_deref())).await?;
    Ok(Json(executions))
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
            permission_mode: payload.permission_mode.unwrap_or_else(|| "ask".to_string()),
        })
        .await?;
    Ok(Json(StartExecutionResponse { execution_id }))
}

async fn get_execution(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<WorkflowExecutionView>, ApiError> {
    let workflow = state.workflow;
    let lookup_id = execution_id.clone();
    let execution = blocking(move || workflow.get_execution_state(&lookup_id))
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("workflow execution '{execution_id}' was not found"))
        })?;
    Ok(Json(workflow_execution_to_view(execution)))
}

async fn get_execution_log(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let workflow = state.workflow;
    let events = blocking(move || workflow.get_execution_log(&execution_id)).await?;
    Ok(Json(events))
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

async fn submit_artifact(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
    payload: Result<Json<SubmitArtifactRequest>, JsonRejection>,
) -> Result<Json<MutationResponse>, ApiError> {
    let Json(payload) = payload.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    state
        .runtime
        .submit_output(SubmitOutputCommand {
            execution_id,
            node_name: payload.node,
            node_execution_id: payload.node_execution_id,
            contract: payload.contract,
            artifact: payload.value,
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
    let validation = blocking(move || {
        workflow.validate_output_for_contract(
            &execution_id,
            &payload.node,
            &payload.contract,
            payload.value,
        )
    })
    .await?;
    Ok(Json(validation.into()))
}

async fn get_artifact(
    State(state): State<LocalApiState>,
    Path((execution_id, node)): Path<(String, String)>,
) -> Result<Json<GetArtifactResponse>, ApiError> {
    let workflow = state.workflow;
    let output = blocking(move || workflow.get_output(&execution_id, &node)).await?;
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
    match value {
        None | Some("api") => Ok(ExecutionOrigin::Api),
        Some("cli") => Ok(ExecutionOrigin::Cli),
        Some("agent") => Ok(ExecutionOrigin::Agent),
        Some(other) => Err(ApiError::invalid_request(format!(
            "invalid created_from value: {other}"
        ))),
    }
}

async fn blocking<T>(
    task: impl FnOnce() -> Result<T, WorkflowError> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| ApiError::internal(format!("workflow query task failed: {error}")))?
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_filter_and_origin_vocabulary() {
        assert_eq!(parse_status_filter(None).unwrap(), None);
        assert_eq!(
            parse_status_filter(Some("active")).unwrap(),
            Some(ExecutionStatusFilter::Active)
        );
        assert_eq!(parse_execution_origin(None).unwrap(), ExecutionOrigin::Api);
        assert_eq!(
            parse_execution_origin(Some("cli")).unwrap(),
            ExecutionOrigin::Cli
        );
        assert!(parse_execution_origin(Some("desktop_ui")).is_err());
    }
}
