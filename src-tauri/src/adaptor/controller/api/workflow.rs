use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::adaptor::presenter::workflow::workflow_execution_to_view;
use crate::adaptor::protocol::workflow::WorkflowExecutionView;
use crate::domain::workflow::{
    ExecutionOrigin, ExecutionStatusFilter, WorkflowError, WorkflowPageRequest,
};
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeExecutionCommand, RetryNodeCommand,
    StartExecutionCommand, StopExecutionCommand, SubmitOutputArtifact, SubmitOutputCommand,
};
use crate::usecase::workflow::dto::{WorkflowExecutionSummaryDto, WorkflowSummaryDto};
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;

use super::error::ApiError;
use super::protocol::{
    ApproveNodeRequest, GetArtifactResponse, MutationResponse, RetryNodeRequest,
    StartExecutionRequest, StartExecutionResponse, SubmitOutputRequest, ValidateArtifactRequest,
    ValidateArtifactResponse,
};
use super::LocalApiState;

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
            "/v1/workflow/executions/{execution_id}/stop",
            post(stop_execution),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/resume",
            post(resume_execution),
        )
        .route(
            "/v1/workflow/executions/{execution_id}/retry",
            post(retry_node),
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
) -> Result<Json<Vec<WorkflowSummaryDto>>, ApiError> {
    let workflow = state.workflow;
    let summaries = blocking(move || workflow.list_workflow_summaries()).await?;
    Ok(Json(summaries))
}

async fn diagnose_workflows(
    State(state): State<LocalApiState>,
    query: Result<Query<DiagnosticsQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let target = WorkflowDiagnosticsTarget::from_optional_directory(query.dir)?;
    let workflow = state.workflow;
    let report = blocking(move || workflow.diagnose_all(target)).await?;
    Ok(Json(report))
}

async fn list_executions(
    State(state): State<LocalApiState>,
    query: Result<Query<ExecutionListQuery>, QueryRejection>,
) -> Result<Json<Vec<WorkflowExecutionSummaryDto>>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let status = parse_status_filter(query.status.as_deref())?;
    let worktree = query.worktree;
    let page = parse_page(query.limit, query.offset)?;
    let workflow = state.workflow;
    let executions =
        blocking(move || workflow.list_executions_filtered(status, worktree.as_deref(), page))
            .await?;
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
    query: Result<Query<ExecutionLogQuery>, QueryRejection>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let Query(query) = query.map_err(|error| ApiError::invalid_request(error.body_text()))?;
    let page = parse_page(query.limit, query.offset)?;
    let workflow = state.workflow;
    let events = blocking(move || workflow.get_execution_log_page(&execution_id, page)).await?;
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

async fn stop_execution(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    state
        .runtime
        .stop_execution(StopExecutionCommand { execution_id })
        .await?;
    Ok(Json(MutationResponse::ok()))
}

async fn resume_execution(
    State(state): State<LocalApiState>,
    Path(execution_id): Path<String>,
) -> Result<Json<MutationResponse>, ApiError> {
    state
        .runtime
        .resume_execution(ResumeExecutionCommand { execution_id })
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
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| ApiError::internal(format!("workflow query task failed: {error}")))?
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::adaptor::controller::api::test_support::{get_json, test_router};

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

    #[test]
    fn validates_page_limits_and_applies_defaults() {
        assert_eq!(
            parse_page(None, None).unwrap(),
            WorkflowPageRequest::new(0, DEFAULT_PAGE_LIMIT as usize)
        );
        assert_eq!(
            parse_page(Some(2), Some(3)).unwrap(),
            WorkflowPageRequest::new(3, 2)
        );
        assert!(parse_page(Some(0), None).is_err());
        assert!(parse_page(Some(MAX_PAGE_LIMIT + 1), None).is_err());
    }

    #[tokio::test]
    async fn test_診断api_queryを検証して共通reportを返す() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        std::fs::write(workflows.path().join("custom.yml"), "name: [").unwrap();
        let (router, _, _) = test_router(data.path(), "secret");

        // When
        let applied = get_json(&router, "/v1/workflow/diagnostics").await;

        // Then
        assert_eq!(applied.0, StatusCode::OK);
        assert!(applied.1["items"].is_array());
        for field in ["workflow_summaries", "facet_summaries", "facet_usage"] {
            assert!(applied.1[field].is_object());
        }

        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(workflows.path().as_os_str().as_encoded_bytes())
                .collect();
        let specified = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;
        assert_eq!(specified.0, StatusCode::OK);
        assert!(specified.1["workflow_summaries"]["custom"].is_object());

        for uri in [
            "/v1/workflow/diagnostics?dir=",
            "/v1/workflow/diagnostics?dir=relative/path",
            "/v1/workflow/diagnostics?unknown=1",
        ] {
            let response = get_json(&router, uri).await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST, "uri: {uri}");
        }

        let unauthorized = router
            .oneshot(
                Request::builder()
                    .uri("/v1/workflow/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_診断api_存在しない指定directoryをnot_foundにする() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let missing = data.path().join("missing");
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(missing.as_os_str().as_encoded_bytes()).collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;

        // Then
        assert_eq!(response.0, StatusCode::NOT_FOUND);
        assert_eq!(response.1["code"], "not_found");
        assert_eq!(
            response.1["message"],
            format!("directory does not exist: {}", missing.display())
        );
        assert!(response.1.get("items").is_none());
    }

    #[tokio::test]
    async fn test_診断api_通常fileの指定をinternal_errorにする() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let file = data.path().join("workflow.yml");
        std::fs::write(&file, "name: workflow").unwrap();
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(file.as_os_str().as_encoded_bytes()).collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;

        // Then
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["code"], "workflow_error");
        assert!(response.1.get("items").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_診断api_列挙不能な指定directoryをinternal_errorにする() {
        use std::os::unix::fs::PermissionsExt;

        // Given
        let data = tempfile::tempdir().unwrap();
        let unreadable = data.path().join("unreadable");
        std::fs::create_dir(&unreadable).unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&unreadable).is_ok() {
            std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!(
                "skipping unreadable directory test because this process can read mode 0o000"
            );
            return;
        }
        let (router, _, _) = test_router(data.path(), "secret");
        let encoded_dir: String =
            url::form_urlencoded::byte_serialize(unreadable.as_os_str().as_encoded_bytes())
                .collect();

        // When
        let response = get_json(
            &router,
            &format!("/v1/workflow/diagnostics?dir={encoded_dir}"),
        )
        .await;
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();

        // Then
        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1["code"], "workflow_error");
        assert!(response.1.get("items").is_none());
    }
}
