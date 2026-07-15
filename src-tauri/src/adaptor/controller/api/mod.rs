mod auth;
mod error;
pub(crate) mod protocol;
mod workflow;

use std::sync::Arc;

use axum::middleware;
use axum::response::IntoResponse;
use axum::Router;

use crate::usecase::workflow::{WorkflowReadUsecase, WorkflowRuntimeUsecase};

#[derive(Clone)]
struct LocalApiState {
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
}

pub(crate) fn build_router(
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
    token: Arc<str>,
) -> Router {
    workflow::router()
        .fallback(|| async {
            error::ApiError::not_found("local API endpoint was not found").into_response()
        })
        .layer(middleware::from_fn_with_state(token, auth::require_bearer))
        .with_state(LocalApiState { workflow, runtime })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind, Workflow as GatewayWorkflow,
    };
    use crate::adaptor::gateway::workflow::storage;
    use crate::adaptor::gateway::workflow::WorkflowEventLogRepository;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, TokenUsage, WorkflowDefinition, WorkflowError,
        WorkflowRuntimeSnapshot,
    };
    use crate::usecase::workflow::command::{
        AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand, SubmitOutputCommand,
    };
    use crate::usecase::workflow::ports::{
        ApprovalChatTarget, WorkflowAbortExecutionGateway, WorkflowApprovalChatGateway,
        WorkflowApprovalGateway, WorkflowEventDraft, WorkflowEventRepository,
        WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway, WorkflowStallClearedCommand,
        WorkflowStallObservedCommand, WorkflowStallObservedGateway, WorkflowStartExecutionGateway,
        WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway,
    };
    use crate::usecase::workflow::WorkflowUsecase;

    use super::*;

    #[derive(Default)]
    pub(crate) struct RecordedRuntimeCommands {
        pub(crate) starts: Vec<ResolvedStartExecutionCommand>,
        pub(crate) approvals: Vec<ApprovalCommand>,
        pub(crate) aborts: Vec<AbortExecutionCommand>,
        pub(crate) outputs: Vec<SubmitOutputCommand>,
    }

    #[derive(Default)]
    struct RecordedRuntimeErrors {
        start: Option<WorkflowError>,
        abort: Option<WorkflowError>,
        approval: Option<WorkflowError>,
        output: Option<WorkflowError>,
    }

    #[derive(Default)]
    pub(crate) struct RecordingRuntimeGateway {
        pub(crate) commands: Mutex<RecordedRuntimeCommands>,
        workflow_resolution: Mutex<Option<(PathBuf, PathBuf)>>,
        errors: Mutex<RecordedRuntimeErrors>,
    }

    impl RecordingRuntimeGateway {
        pub(crate) fn resolve_workflows_from(
            &self,
            workflows_dir: PathBuf,
            facets_base_dir: PathBuf,
        ) {
            *self.workflow_resolution.lock().unwrap() = Some((workflows_dir, facets_base_dir));
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStartExecutionGateway for RecordingRuntimeGateway {
        async fn resolve_start_execution_worktree(
            &self,
            worktree_path: String,
        ) -> Result<String, WorkflowError> {
            Ok(worktree_path)
        }

        async fn resolve_start_execution_workflow(
            &self,
            workflow_name: &str,
        ) -> Result<WorkflowDefinition, WorkflowError> {
            let resolution = self.workflow_resolution.lock().unwrap().clone();
            let Some((workflows_dir, facets_base_dir)) = resolution else {
                return Ok(WorkflowDefinition::default());
            };
            let workflow = crate::adaptor::gateway::workflow::resolve_workflow_by_name(
                &workflows_dir,
                &facets_base_dir,
                workflow_name,
            )
            .map_err(|error| match error {
                crate::adaptor::gateway::workflow::resolver::WorkflowDefinitionResolverError::InvalidWorkflow(
                    message,
                ) => WorkflowError::validation(message),
                crate::adaptor::gateway::workflow::resolver::WorkflowDefinitionResolverError::Infrastructure(
                    message,
                ) => WorkflowError::external(message),
            })?;
            crate::adaptor::gateway::workflow::mapper::schema_workflow_to_domain(workflow)
        }

        async fn start_resolved_execution(
            &self,
            command: ResolvedStartExecutionCommand,
        ) -> Result<String, WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().start.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().starts.push(command);
            Ok("00000000-0000-4000-8000-000000000001".to_string())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowAbortExecutionGateway for RecordingRuntimeGateway {
        async fn abort_execution(
            &self,
            command: AbortExecutionCommand,
        ) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().abort.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().aborts.push(command);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowApprovalGateway for RecordingRuntimeGateway {
        async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().approval.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().approvals.push(command);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowSubmitOutputGateway for RecordingRuntimeGateway {
        async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().output.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().outputs.push(command);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowTurnCompleteGateway for RecordingRuntimeGateway {
        async fn is_session_running(&self, _chat_session_id: &str) -> bool {
            false
        }

        async fn complete_turn(
            &self,
            _command: WorkflowTurnCompleteCommand,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowStallObservedGateway for RecordingRuntimeGateway {
        async fn observe_stall(
            &self,
            _command: WorkflowStallObservedCommand,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }

        async fn clear_stall(
            &self,
            _command: WorkflowStallClearedCommand,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRuntimeStateGateway for RecordingRuntimeGateway {
        async fn get_state_by_execution_id(
            &self,
            _execution_id: &str,
        ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
            Ok(None)
        }

        async fn get_state_by_worktree(
            &self,
            _worktree_path: &str,
        ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl WorkflowRuntimeShutdownGateway for RecordingRuntimeGateway {
        async fn shutdown_active_commands(&self) {}
    }

    #[async_trait::async_trait]
    impl WorkflowApprovalChatGateway for RecordingRuntimeGateway {
        async fn resolve_approval_chat_target(
            &self,
            _execution_id: &str,
        ) -> Result<ApprovalChatTarget, WorkflowError> {
            Ok(ApprovalChatTarget {
                chat_session_id: "chat".to_string(),
                worktree_path: "/tmp/worktree".to_string(),
            })
        }

        async fn validate_approval_chat_instruction(
            &self,
            _chat_session_id: &str,
            _content: &str,
        ) -> Result<(), WorkflowError> {
            Ok(())
        }
    }

    pub(crate) fn usecases(
        data_dir: &Path,
    ) -> (
        Arc<WorkflowUsecase>,
        Arc<WorkflowRuntimeUsecase>,
        Arc<RecordingRuntimeGateway>,
    ) {
        let workflow = Arc::new(crate::adaptor::controller::wiring::build_workflow_usecase(
            data_dir,
        ));
        let gateway = Arc::new(RecordingRuntimeGateway::default());
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(gateway.clone()));
        (workflow, runtime, gateway)
    }

    pub(crate) fn test_router(
        data_dir: &Path,
        token: &str,
    ) -> (
        Router,
        Arc<WorkflowRuntimeUsecase>,
        Arc<RecordingRuntimeGateway>,
    ) {
        let (workflow, runtime, gateway) = usecases(data_dir);
        let router = build_router(
            Arc::new(workflow.read_usecase()),
            runtime.clone(),
            Arc::<str>::from(token),
        );
        (router, runtime, gateway)
    }

    pub(crate) async fn send_json(
        router: &Router,
        uri: &str,
        payload: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = serde_json::from_slice(&body).unwrap();
        (status, body)
    }

    async fn get_json(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
        let body = serde_json::from_slice(&body).unwrap();
        (status, body)
    }

    pub(crate) fn seed_query_execution(data_dir: &Path, execution_id: &str) {
        let executions_dir = data_dir.join("workflow_executions");
        fs::create_dir_all(&executions_dir).unwrap();
        let metadata = WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::Cli,
            started_at: 100.0,
            updated_at: 101.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: TokenUsage::default(),
        };
        fs::write(
            executions_dir.join(format!("{execution_id}.json")),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();

        WorkflowEventLogRepository::new(data_dir)
            .append(&WorkflowEventDraft {
                execution_id: execution_id.to_string(),
                event_kind: "execution_started".to_string(),
                timestamp: 100.0,
                payload: serde_json::json!({
                    "workflow_name": "review",
                    "worktree_path": "/repo",
                    "created_from": "cli",
                    "request": "review this change",
                    "definition": {
                        "name": "review",
                        "description": "Review a change",
                        "builtin": false,
                        "schemas": {
                            "review-result": {
                                "type": "object",
                                "properties": {
                                    "status": {"type": "string"}
                                },
                                "required": ["status"],
                                "additionalProperties": false
                            }
                        },
                        "nodes": [{
                            "name": "review",
                            "session": {"gate": "auto"},
                            "artifact": "review-result"
                        }]
                    }
                }),
            })
            .unwrap();
    }

    pub(crate) fn seed_submitted_output(data_dir: &Path, execution_id: &str) {
        let events = WorkflowEventLogRepository::new(data_dir);
        events
            .append(&WorkflowEventDraft {
                execution_id: execution_id.to_string(),
                event_kind: "node_started".to_string(),
                timestamp: 105.0,
                payload: serde_json::json!({
                    "node_execution_id": "ne-review-1",
                    "node_name": "review",
                    "kind": "session",
                    "attempt": 1
                }),
            })
            .unwrap();
        events
            .append(&WorkflowEventDraft {
                execution_id: execution_id.to_string(),
                event_kind: "artifact_produced".to_string(),
                timestamp: 110.0,
                payload: serde_json::json!({
                    "node_execution_id": "ne-review-1",
                    "node_name": "review",
                    "contract": "review-result",
                    "value": {"status": "approved"},
                    "submitted_at": 109.0,
                    "request_id": "request-1"
                }),
            })
            .unwrap();
    }

    #[tokio::test]
    async fn start_resolves_definition_name_and_reports_duplicate_name_diagnostics() {
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        let definition = GatewayWorkflow {
            name: "declared-name".to_string(),
            description: "API start fixture".to_string(),
            nodes: vec![NodeDefinition {
                name: "execute".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..NodeDefinition::default()
            }],
            ..GatewayWorkflow::default()
        };
        storage::save_workflow(workflows.path(), &definition).unwrap();
        fs::rename(
            workflows.path().join("declared-name.yml"),
            workflows.path().join("different-filename.yml"),
        )
        .unwrap();

        let (router, _, gateway) = test_router(data.path(), "secret");
        gateway.resolve_workflows_from(
            workflows.path().to_path_buf(),
            workflows.path().to_path_buf(),
        );
        let start = send_json(
            &router,
            "/v1/workflow/executions",
            serde_json::json!({
                "workflow_name": "declared-name",
                "worktree_path": "/repo",
                "request": "API request artifact",
                "created_from": "cli"
            }),
        )
        .await;
        assert_eq!(start.0, StatusCode::OK);
        {
            let commands = gateway.commands.lock().unwrap();
            assert_eq!(commands.starts[0].workflow.name, "declared-name");
            assert_eq!(
                commands.starts[0].request.as_deref(),
                Some("API request artifact")
            );
        }

        fs::copy(
            workflows.path().join("different-filename.yml"),
            workflows.path().join("duplicate-name.yml"),
        )
        .unwrap();
        let duplicate = send_json(
            &router,
            "/v1/workflow/executions",
            serde_json::json!({
                "workflow_name": "declared-name",
                "worktree_path": "/repo",
                "request": ""
            }),
        )
        .await;
        assert_eq!(duplicate.0, StatusCode::BAD_REQUEST);
        assert_eq!(duplicate.1["code"], "validation_error");
        assert!(duplicate.1["message"].as_str().unwrap().contains("WFS006"));
        assert_eq!(gateway.commands.lock().unwrap().starts.len(), 1);
    }

    #[tokio::test]
    async fn every_endpoint_and_the_fallback_require_the_bearer_token() {
        let directory = tempfile::tempdir().unwrap();
        let (router, _, _) = test_router(directory.path(), "secret");
        let execution_id = "00000000-0000-4000-8000-000000000001";
        let endpoints = [
            (Method::GET, "/v1/workflows".to_string()),
            (Method::GET, "/v1/workflow/executions".to_string()),
            (Method::POST, "/v1/workflow/executions".to_string()),
            (
                Method::GET,
                format!("/v1/workflow/executions/{execution_id}"),
            ),
            (
                Method::GET,
                format!("/v1/workflow/executions/{execution_id}/log"),
            ),
            (
                Method::POST,
                format!("/v1/workflow/executions/{execution_id}/approve"),
            ),
            (
                Method::POST,
                format!("/v1/workflow/executions/{execution_id}/abort"),
            ),
            (
                Method::POST,
                format!("/v1/workflow/executions/{execution_id}/artifacts"),
            ),
            (
                Method::POST,
                format!("/v1/workflow/executions/{execution_id}/artifacts:validate"),
            ),
            (
                Method::GET,
                format!("/v1/workflow/executions/{execution_id}/artifacts/review"),
            ),
            (Method::GET, "/not-an-endpoint".to_string()),
        ];

        for (method, uri) in endpoints {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "endpoint did not require auth: {uri}"
            );
        }
    }

    #[tokio::test]
    async fn authenticated_mutation_endpoints_delegate_typed_commands_to_the_runtime_usecase() {
        let directory = tempfile::tempdir().unwrap();
        let (router, _, gateway) = test_router(directory.path(), "secret");
        let execution_id = "00000000-0000-4000-8000-000000000123";
        let start = send_json(
            &router,
            "/v1/workflow/executions",
            serde_json::json!({
                "workflow_name": "review",
                "worktree_path": "/repo",
                "request": "review this change",
                "permission_mode": "edit",
                "created_from": "agent"
            }),
        )
        .await;
        assert_eq!(start.0, StatusCode::OK);
        assert_eq!(
            start.1,
            serde_json::json!({
                "execution_id": "00000000-0000-4000-8000-000000000001"
            })
        );

        let approve = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/approve"),
            serde_json::json!({
                "node": "review",
                "node_execution_id": "ne-review-2",
                "comment": "looks good"
            }),
        )
        .await;
        assert_eq!(approve, (StatusCode::OK, serde_json::json!({"ok": true})));

        let abort = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/workflow/executions/{execution_id}/abort"))
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(abort.status(), StatusCode::OK);

        let submit = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts"),
            serde_json::json!({
                "node": "review",
                "node_execution_id": "ne-review-2",
                "contract": "review-result",
                "value": {"status": "approved"}
            }),
        )
        .await;
        assert_eq!(submit, (StatusCode::OK, serde_json::json!({"ok": true})));

        let commands = gateway.commands.lock().unwrap();
        assert_eq!(commands.starts.len(), 1);
        assert_eq!(commands.starts[0].workflow_name, "review");
        assert_eq!(commands.starts[0].worktree_path, "/repo");
        assert_eq!(
            commands.starts[0].request.as_deref(),
            Some("review this change")
        );
        assert_eq!(
            commands.starts[0].created_from,
            crate::domain::workflow::ExecutionOrigin::Agent
        );
        assert_eq!(commands.starts[0].permission_mode, "edit");

        assert_eq!(commands.approvals.len(), 1);
        assert_eq!(commands.approvals[0].execution_id, execution_id);
        assert_eq!(commands.approvals[0].node_name, "review");
        assert_eq!(
            commands.approvals[0].node_execution_id.as_deref(),
            Some("ne-review-2")
        );
        assert_eq!(commands.approvals[0].comment.as_deref(), Some("looks good"));

        assert_eq!(commands.aborts.len(), 1);
        assert_eq!(commands.aborts[0].execution_id, execution_id);
        assert_eq!(commands.aborts[0].expected_node_name, None);

        assert_eq!(commands.outputs.len(), 1);
        assert_eq!(commands.outputs[0].execution_id, execution_id);
        assert_eq!(commands.outputs[0].node_name, "review");
        assert_eq!(
            commands.outputs[0].node_execution_id.as_deref(),
            Some("ne-review-2")
        );
        assert_eq!(commands.outputs[0].contract, "review-result");
        assert_eq!(
            commands.outputs[0].artifact,
            serde_json::json!({"status": "approved"})
        );
    }

    #[tokio::test]
    async fn mutation_endpoints_preserve_typed_domain_error_statuses() {
        let directory = tempfile::tempdir().unwrap();
        let (router, _, gateway) = test_router(directory.path(), "secret");
        let execution_id = "00000000-0000-4000-8000-000000000123";

        gateway.errors.lock().unwrap().start = Some(WorkflowError::validation("invalid start"));
        let start = send_json(
            &router,
            "/v1/workflow/executions",
            serde_json::json!({
                "workflow_name": "review",
                "worktree_path": "/repo",
                "request": "review this change"
            }),
        )
        .await;
        assert_eq!(start.0, StatusCode::BAD_REQUEST);
        assert_eq!(start.1["code"], "validation_error");

        gateway.errors.lock().unwrap().abort =
            Some(WorkflowError::invalid_state("execution is terminal"));
        let abort = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/abort"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(abort.0, StatusCode::CONFLICT);
        assert_eq!(abort.1["code"], "invalid_state");

        gateway.errors.lock().unwrap().approval = Some(WorkflowError::UnauthorizedApprovalTarget(
            "wrong approval target".to_string(),
        ));
        let approval = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/approve"),
            serde_json::json!({"node": "review"}),
        )
        .await;
        assert_eq!(approval.0, StatusCode::FORBIDDEN);
        assert_eq!(approval.1["code"], "unauthorized_approval_target");

        gateway.errors.lock().unwrap().output =
            Some(WorkflowError::NotFound("execution not found".to_string()));
        let output = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts"),
            serde_json::json!({
                "node": "review",
                "contract": "review-result",
                "value": {"status": "approved"}
            }),
        )
        .await;
        assert_eq!(output.0, StatusCode::NOT_FOUND);
        assert_eq!(output.1["code"], "not_found");
    }

    #[tokio::test]
    async fn authenticated_query_endpoints_project_seeded_execution_and_artifact_data() {
        let directory = tempfile::tempdir().unwrap();
        let execution_id = "00000000-0000-4000-8000-000000000321";
        seed_query_execution(directory.path(), execution_id);
        let (router, _, _) = test_router(directory.path(), "secret");

        let executions = get_json(&router, "/v1/workflow/executions?status=active").await;
        assert_eq!(executions.0, StatusCode::OK);
        assert_eq!(executions.1[0]["executionId"], execution_id);
        assert_eq!(executions.1[0]["workflowName"], "review");
        assert_eq!(executions.1[0]["status"], "running");
        assert_eq!(executions.1[0]["createdFrom"], "cli");

        let status = get_json(&router, &format!("/v1/workflow/executions/{execution_id}")).await;
        assert_eq!(status.0, StatusCode::OK);
        assert_eq!(status.1["id"], execution_id);
        assert_eq!(status.1["workflowName"], "review");
        assert_eq!(status.1["status"], "running");
        assert_eq!(status.1["artifacts"][0]["nodeName"], "request");
        assert_eq!(status.1["artifacts"][0]["value"], "review this change");

        let logs = get_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/log"),
        )
        .await;
        assert_eq!(logs.0, StatusCode::OK);
        assert_eq!(logs.1[0]["event"], "execution_started");
        assert_eq!(logs.1[0]["execution_id"], execution_id);
        assert_eq!(logs.1[0]["request"], "review this change");

        let missing_execution_id = "00000000-0000-4000-8000-000000000999";
        let missing_logs = get_json(
            &router,
            &format!("/v1/workflow/executions/{missing_execution_id}/log"),
        )
        .await;
        assert_eq!(missing_logs.0, StatusCode::NOT_FOUND);
        assert_eq!(missing_logs.1["code"], "not_found");

        let validation = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts:validate"),
            serde_json::json!({
                "node": "review",
                "contract": "review-result",
                "value": {"status": "approved"}
            }),
        )
        .await;
        assert_eq!(
            validation,
            (StatusCode::OK, serde_json::json!({"status": "valid"}))
        );

        seed_submitted_output(directory.path(), execution_id);
        let output = get_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts/review"),
        )
        .await;
        assert_eq!(output.0, StatusCode::OK);
        assert_eq!(output.1["status"], "submitted");
        assert_eq!(output.1["contract"], "review-result");
        assert_eq!(
            output.1["structured_output"],
            serde_json::json!({"status": "approved"})
        );
        assert_eq!(output.1["submitted_at"], 109.0);
        assert_eq!(output.1["request_id"], "request-1");
    }

    #[tokio::test]
    async fn execution_and_log_queries_apply_validated_page_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let first_execution_id = "00000000-0000-4000-8000-000000000321";
        let second_execution_id = "00000000-0000-4000-8000-000000000322";
        seed_query_execution(directory.path(), first_execution_id);
        seed_query_execution(directory.path(), second_execution_id);
        seed_submitted_output(directory.path(), first_execution_id);
        let worktree = directory.path().join("worktree");
        fs::create_dir(&worktree).unwrap();
        let canonical_worktree = fs::canonicalize(&worktree).unwrap();
        for execution_id in [first_execution_id, second_execution_id] {
            let metadata_path = directory
                .path()
                .join("workflow_executions")
                .join(format!("{execution_id}.json"));
            let mut metadata: WorkflowExecutionMetadata =
                serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
            metadata.worktree_path = canonical_worktree.to_string_lossy().into_owned();
            fs::write(metadata_path, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
        }
        let (router, _, _) = test_router(directory.path(), "secret");
        let encoded_worktree: String =
            url::form_urlencoded::byte_serialize(canonical_worktree.to_string_lossy().as_bytes())
                .collect();

        let executions = get_json(
            &router,
            &format!(
                "/v1/workflow/executions?status=active&worktree={encoded_worktree}&limit=1&offset=1"
            ),
        )
        .await;
        assert_eq!(executions.0, StatusCode::OK);
        assert_eq!(executions.1.as_array().unwrap().len(), 1);
        assert_eq!(executions.1[0]["status"], "running");
        assert_eq!(
            executions.1[0]["worktreePath"],
            canonical_worktree.to_string_lossy().as_ref()
        );

        let logs = get_json(
            &router,
            &format!("/v1/workflow/executions/{first_execution_id}/log?limit=1&offset=1"),
        )
        .await;
        assert_eq!(logs.0, StatusCode::OK);
        assert_eq!(logs.1.as_array().unwrap().len(), 1);
        assert_eq!(logs.1[0]["event"], "node_started");

        for uri in [
            "/v1/workflow/executions?limit=0",
            "/v1/workflow/executions?limit=201",
            &format!("/v1/workflow/executions/{first_execution_id}/log?offset=-1"),
        ] {
            let response = get_json(&router, uri).await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST, "uri: {uri}");
            assert_eq!(response.1["code"], "invalid_request");
        }
    }
}
