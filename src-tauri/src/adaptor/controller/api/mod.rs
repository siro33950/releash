mod agent_session;
mod auth;
mod error;
pub(crate) mod protocol;
mod terminal;
mod workflow;

pub(crate) use terminal::TerminalApiDeps;

use std::sync::Arc;

use axum::middleware;
use axum::response::IntoResponse;
use axum::Router;

use crate::usecase::workflow::{WorkflowReadUsecase, WorkflowRuntimeUsecase};

const MAX_AGENT_SESSION_CONNECTIONS: usize = 16;

#[derive(Clone)]
struct LocalApiState {
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
    agent_session: Option<AgentSessionApiDeps>,
    terminal: Option<TerminalApiDeps>,
}

#[derive(Clone)]
pub(crate) struct AgentSessionApiDeps {
    pub(crate) send: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    pub(crate) permission_response:
        Arc<crate::usecase::agent_session::operation::PermissionResponseOperationUsecase>,
    pub(crate) stop: Arc<crate::usecase::agent_session::operation::StopOperationUsecase>,
    pub(crate) recovery: Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>,
    pub(crate) feedback: Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>,
    pub(crate) feedback_load:
        Arc<crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase>,
    pub(crate) shutdown: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
    pub(crate) process_actions:
        Arc<crate::adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher>,
    pub(crate) local_store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    pub(crate) caller_journal: Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>,
    pub(crate) app: tauri::AppHandle,
    pub(crate) connection_limit: Arc<tokio::sync::Semaphore>,
}

impl AgentSessionApiDeps {
    #[allow(clippy::too_many_arguments)] // Composition root injects each independent durable service explicitly.
    pub(crate) fn new(
        send: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
        permission_response: Arc<
            crate::usecase::agent_session::operation::PermissionResponseOperationUsecase,
        >,
        stop: Arc<crate::usecase::agent_session::operation::StopOperationUsecase>,
        recovery: Arc<crate::usecase::agent_session::operation::RecoveryActionUsecase>,
        feedback: Arc<crate::usecase::agent_session::feedback::SessionFeedbackUsecase>,
        feedback_load: Arc<
            crate::usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase,
        >,
        shutdown: Arc<crate::usecase::shutdown_coordinator::ShutdownCoordinator>,
        process_actions: Arc<
            crate::adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher,
        >,
        local_store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        caller_journal: Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>,
        app: tauri::AppHandle,
    ) -> Self {
        Self {
            send,
            permission_response,
            stop,
            recovery,
            feedback,
            feedback_load,
            shutdown,
            process_actions,
            local_store,
            caller_journal,
            app,
            connection_limit: Arc::new(tokio::sync::Semaphore::new(MAX_AGENT_SESSION_CONNECTIONS)),
        }
    }
}

pub(crate) fn build_router(
    workflow: Arc<WorkflowReadUsecase>,
    runtime: Arc<WorkflowRuntimeUsecase>,
    token: Arc<str>,
    agent_session: Option<AgentSessionApiDeps>,
    terminal: Option<TerminalApiDeps>,
) -> Router {
    workflow::router()
        .merge(agent_session::router())
        .merge(terminal::router())
        .fallback(|| async {
            error::ApiError::not_found("local API endpoint was not found").into_response()
        })
        .layer(middleware::from_fn_with_state(token, auth::require_bearer))
        .with_state(LocalApiState {
            workflow,
            runtime,
            agent_session,
            terminal,
        })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata;
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind,
        WorkflowDefinitionYaml as GatewayWorkflowDefinitionYaml,
    };
    use crate::adaptor::gateway::workflow::storage;
    use crate::domain::workflow::{
        ExecutionOrigin, ExecutionStatus, TokenUsage, WorkflowDefinition, WorkflowError,
        WorkflowRuntimeSnapshot,
    };
    use crate::usecase::workflow::command::{
        AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand,
        ResumeExecutionCommand, StopExecutionCommand, SubmitOutputCommand,
    };
    use crate::usecase::workflow::ports::{
        ApprovalChatTarget, WorkflowAbortExecutionGateway, WorkflowApprovalChatGateway,
        WorkflowApprovalGateway, WorkflowEventDraft, WorkflowResumeExecutionGateway,
        WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway, WorkflowStallClearedCommand,
        WorkflowStallObservedCommand, WorkflowStallObservedGateway, WorkflowStartExecutionGateway,
        WorkflowStopExecutionGateway, WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand,
        WorkflowTurnCompleteGateway,
    };
    use crate::usecase::workflow::WorkflowUsecase;

    use super::*;

    #[derive(Default)]
    pub(crate) struct RecordedRuntimeCommands {
        pub(crate) starts: Vec<ResolvedStartExecutionCommand>,
        pub(crate) approvals: Vec<ApprovalCommand>,
        pub(crate) aborts: Vec<AbortExecutionCommand>,
        pub(crate) stops: Vec<StopExecutionCommand>,
        pub(crate) resumes: Vec<ResumeExecutionCommand>,
        pub(crate) outputs: Vec<SubmitOutputCommand>,
    }

    #[derive(Default)]
    struct RecordedRuntimeErrors {
        start: Option<WorkflowError>,
        abort: Option<WorkflowError>,
        stop: Option<WorkflowError>,
        resume: Option<WorkflowError>,
        approval: Option<WorkflowError>,
        output: Option<WorkflowError>,
    }

    #[derive(Default)]
    pub(crate) struct RecordingRuntimeGateway {
        pub(crate) commands: Mutex<RecordedRuntimeCommands>,
        workflow_resolution: Mutex<Option<(PathBuf, PathBuf)>>,
        output_persistence_data_dir: Mutex<Option<PathBuf>>,
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

        fn persist_submitted_outputs_to(&self, data_dir: PathBuf) {
            *self.output_persistence_data_dir.lock().unwrap() = Some(data_dir);
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
                crate::usecase::workflow::runtime_resolver::WorkflowDefinitionResolverError::InvalidWorkflow(
                    message,
                ) => WorkflowError::validation(message),
                crate::usecase::workflow::runtime_resolver::WorkflowDefinitionResolverError::Infrastructure(
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
    impl WorkflowStopExecutionGateway for RecordingRuntimeGateway {
        async fn stop_execution(&self, command: StopExecutionCommand) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().stop.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().stops.push(command);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl WorkflowResumeExecutionGateway for RecordingRuntimeGateway {
        async fn resume_execution(
            &self,
            command: ResumeExecutionCommand,
        ) -> Result<(), WorkflowError> {
            if let Some(error) = self.errors.lock().unwrap().resume.clone() {
                return Err(error);
            }
            self.commands.lock().unwrap().resumes.push(command);
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
            if let Some(data_dir) = self.output_persistence_data_dir.lock().unwrap().clone() {
                append_canonical_workflow_drafts(
                    &data_dir,
                    &[WorkflowEventDraft {
                        execution_id: command.execution_id.clone(),
                        event_kind: "artifact_produced".to_string(),
                        timestamp: 110.0,
                        payload: serde_json::json!({
                            "node_execution_id": command.node_execution_id,
                            "node_name": command.node_name,
                            "contract": command.contract,
                            "value": command.artifact,
                            "submitted_at": 109.0,
                            "request_id": "request-1"
                        }),
                    }],
                )?;
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

        async fn application_shutdown_target_execution_ids(&self) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
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
        test_router_with_optional_terminal(data_dir, token, None)
    }

    pub(crate) fn test_router_with_terminal(
        data_dir: &Path,
        token: &str,
        terminal: TerminalApiDeps,
    ) -> Router {
        test_router_with_optional_terminal(data_dir, token, Some(terminal)).0
    }

    fn test_router_with_optional_terminal(
        data_dir: &Path,
        token: &str,
        terminal: Option<TerminalApiDeps>,
    ) -> (
        Router,
        Arc<WorkflowRuntimeUsecase>,
        Arc<RecordingRuntimeGateway>,
    ) {
        let gateway = Arc::new(RecordingRuntimeGateway::default());
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(gateway.clone()));
        let workflow = crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            data_dir, None,
        )
        .or_else(|read_error| {
            // A fresh API fixture has no DB yet. Initialize it once, but
            // never contend with an already-managed production writer.
            if data_dir
                .join(crate::adaptor::gateway::local_event_store::layout::DATABASE_FILE)
                .exists()
            {
                return Err(read_error);
            }
            drop(
                LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                    .map_err(|error| error.to_string())?,
            );
            crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
                data_dir, None,
            )
        })
        .unwrap();
        let router = build_router(
            Arc::new(workflow),
            runtime.clone(),
            Arc::<str>::from(token),
            None,
            terminal,
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

    pub(crate) async fn get_json(router: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
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

    fn canonical_local_event_store(data_dir: &Path) -> Arc<LocalEventStore> {
        LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf())).unwrap()
    }

    fn append_canonical_workflow_drafts(
        data_dir: &Path,
        drafts: &[WorkflowEventDraft],
    ) -> Result<(), WorkflowError> {
        let events = drafts
            .iter()
            .map(crate::adaptor::gateway::workflow::mapper::event_draft_to_event)
            .collect::<Result<Vec<_>, _>>()?;
        crate::adaptor::gateway::workflow::test_support::append_canonical_events(
            &canonical_local_event_store(data_dir),
            &events,
        )
        .map_err(WorkflowError::external)
    }

    pub(crate) fn seed_query_execution(data_dir: &Path, execution_id: &str) {
        seed_query_execution_at(data_dir, execution_id, "/repo");
    }

    fn seed_query_execution_at(data_dir: &Path, execution_id: &str, worktree_path: &str) {
        let metadata = WorkflowExecutionMetadata {
            execution_id: execution_id.to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: worktree_path.to_string(),
            current_node: Some("review".to_string()),
            created_from: ExecutionOrigin::Cli,
            started_at: 100.0,
            updated_at: 101.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        };
        let started = WorkflowEventDraft {
            execution_id: execution_id.to_string(),
            event_kind: "execution_started".to_string(),
            timestamp: 100.0,
            payload: serde_json::json!({
                "workflow_name": "review",
                "worktree_path": worktree_path,
                "created_from": "cli",
                "request": "review this change",
                "permission_mode": "ask",
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
                            "required": ["status"]
                        }
                    },
                    "nodes": [{
                        "name": "review",
                        "session": {"gate": "auto"},
                        "artifact": "review-result"
                    }]
                }
            }),
        };
        let event =
            crate::adaptor::gateway::workflow::mapper::event_draft_to_event(&started).unwrap();
        crate::adaptor::gateway::workflow::test_support::seed_canonical_execution(
            &canonical_local_event_store(data_dir),
            &metadata,
            &[event],
        );
    }

    pub(crate) fn seed_submitted_output(data_dir: &Path, execution_id: &str) {
        let drafts = [
            WorkflowEventDraft {
                execution_id: execution_id.to_string(),
                event_kind: "node_started".to_string(),
                timestamp: 105.0,
                payload: serde_json::json!({
                    "node_execution_id": "ne-review-1",
                    "node_name": "review",
                    "kind": "session",
                    "attempt": 1
                }),
            },
            WorkflowEventDraft {
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
            },
        ];
        append_canonical_workflow_drafts(data_dir, &drafts).unwrap();
    }

    #[tokio::test]
    async fn start_resolves_definition_name_and_reports_duplicate_name_diagnostics() {
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        let definition = GatewayWorkflowDefinitionYaml {
            name: "declared-name".to_string(),
            description: "API start fixture".to_string(),
            nodes: vec![NodeDefinition {
                name: "execute".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..NodeDefinition::default()
            }],
            ..GatewayWorkflowDefinitionYaml::default()
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
            (Method::GET, "/v1/agent-session".to_string()),
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
                format!("/v1/workflow/executions/{execution_id}/stop"),
            ),
            (
                Method::POST,
                format!("/v1/workflow/executions/{execution_id}/resume"),
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

        let stop = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/workflow/executions/{execution_id}/stop"))
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stop.status(), StatusCode::OK);

        let resume = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/v1/workflow/executions/{execution_id}/resume"))
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resume.status(), StatusCode::OK);

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

        assert_eq!(commands.stops.len(), 1);
        assert_eq!(commands.stops[0].execution_id, execution_id);

        assert_eq!(commands.resumes.len(), 1);
        assert_eq!(commands.resumes[0].execution_id, execution_id);

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

        gateway.errors.lock().unwrap().stop =
            Some(WorkflowError::invalid_state("execution cannot be stopped"));
        let stop = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/stop"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(stop.0, StatusCode::CONFLICT);
        assert_eq!(stop.1["code"], "invalid_state");

        gateway.errors.lock().unwrap().resume =
            Some(WorkflowError::NotFound("execution not found".to_string()));
        let resume = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/resume"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(resume.0, StatusCode::NOT_FOUND);
        assert_eq!(resume.1["code"], "not_found");

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
    async fn stop_and_resume_reject_invalid_execution_ids_before_the_runtime_gateway() {
        let directory = tempfile::tempdir().unwrap();
        let (router, _, gateway) = test_router(directory.path(), "secret");

        for action in ["stop", "resume"] {
            let response = send_json(
                &router,
                &format!("/v1/workflow/executions/not-a-uuid/{action}"),
                serde_json::json!({}),
            )
            .await;
            assert_eq!(response.0, StatusCode::BAD_REQUEST);
            assert_eq!(response.1["code"], "validation_error");
        }

        let commands = gateway.commands.lock().unwrap();
        assert!(commands.stops.is_empty());
        assert!(commands.resumes.is_empty());
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
        assert_eq!(output.1["value"], serde_json::json!({"status": "approved"}));
        assert_eq!(output.1["submitted_at"], 109.0);
        assert_eq!(output.1["request_id"], "request-1");
    }

    #[tokio::test]
    async fn submitted_artifact_round_trips_through_persistence_and_get_wire_response() {
        let directory = tempfile::tempdir().unwrap();
        let execution_id = "00000000-0000-4000-8000-000000000323";
        seed_query_execution(directory.path(), execution_id);
        append_canonical_workflow_drafts(
            directory.path(),
            &[WorkflowEventDraft {
                execution_id: execution_id.to_string(),
                event_kind: "node_started".to_string(),
                timestamp: 105.0,
                payload: serde_json::json!({
                    "node_execution_id": "ne-review-1",
                    "node_name": "review",
                    "kind": "session",
                    "attempt": 1
                }),
            }],
        )
        .unwrap();
        let (router, _, gateway) = test_router(directory.path(), "secret");
        gateway.persist_submitted_outputs_to(directory.path().to_path_buf());

        let submit = send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts"),
            serde_json::json!({
                "node": "review",
                "node_execution_id": "ne-review-1",
                "contract": "review-result",
                "value": {"status": "approved"}
            }),
        )
        .await;
        assert_eq!(submit, (StatusCode::OK, serde_json::json!({"ok": true})));

        let output = get_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/artifacts/review"),
        )
        .await;
        assert_eq!(output.0, StatusCode::OK);
        assert_eq!(output.1["status"], "submitted");
        assert_eq!(output.1["value"], serde_json::json!({"status": "approved"}));
        assert!(output.1.get("structured_output").is_none());
    }

    #[tokio::test]
    async fn execution_and_log_queries_apply_validated_page_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let first_execution_id = "00000000-0000-4000-8000-000000000321";
        let second_execution_id = "00000000-0000-4000-8000-000000000322";
        let (repository, git_repository) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&git_repository);
        let canonical_worktree = repository
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut config = crate::adaptor::gateway::app_config::ReleashConfig::default();
        config.app.last_repo_paths = vec![canonical_worktree.clone()];
        crate::adaptor::gateway::app_config::repository_impl::write_config(
            &directory.path().join("releash.toml"),
            &config,
        )
        .unwrap();
        seed_query_execution_at(directory.path(), first_execution_id, &canonical_worktree);
        seed_query_execution_at(directory.path(), second_execution_id, &canonical_worktree);
        seed_submitted_output(directory.path(), first_execution_id);
        let (router, _, _) = test_router(directory.path(), "secret");
        let encoded_worktree: String =
            url::form_urlencoded::byte_serialize(canonical_worktree.as_bytes()).collect();

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
        assert_eq!(executions.1[0]["worktreePath"], canonical_worktree);

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
