use std::path::Path;

use super::common::CliError;
use crate::adaptor::controller::api::protocol::{
    ApproveNodeRequest, GetArtifactResponse, MutationResponse, StartExecutionRequest,
    StartExecutionResponse, SubmitArtifactRequest, ValidateArtifactRequest,
    ValidateArtifactResponse,
};
use crate::infrastructure::local_api::{LocalApiClientError, LocalApiHttpClient};
use crate::usecase::workflow::dto::{WorkflowExecutionSummaryDto, WorkflowSummaryDto};
use crate::usecase::workflow::{WorkflowGetOutputResult, WorkflowValidateOutputResult};

const NODE_EXECUTION_ID_ENV: &str = "RELEASH_NODE_EXECUTION_ID";
const RECONCILIATION_PAGE_LIMIT: usize = 100;

#[derive(Debug)]
pub(super) enum ApiRequestError {
    Unavailable,
    Cli(CliError),
}

impl From<CliError> for ApiRequestError {
    fn from(error: CliError) -> Self {
        Self::Cli(error)
    }
}

impl From<LocalApiClientError> for ApiRequestError {
    fn from(error: LocalApiClientError) -> Self {
        match error {
            LocalApiClientError::Unavailable(_) => Self::Unavailable,
            LocalApiClientError::HttpStatus { status, message } => {
                Self::Cli(api_error(status, message.as_deref()))
            }
            error => Self::Cli(CliError::Other(error.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct LocalApiClient {
    transport: LocalApiHttpClient,
}

impl LocalApiClient {
    fn discover(data_dir: &Path) -> Result<Option<Self>, CliError> {
        LocalApiHttpClient::discover(data_dir)
            .map(|client| client.map(|transport| Self { transport }))
            .map_err(|error| CliError::Other(error.to_string()))
    }

    pub(super) fn workflows(&self) -> Result<Vec<WorkflowSummaryDto>, ApiRequestError> {
        self.transport
            .get_json(&["v1", "workflows"], &[])
            .map_err(ApiRequestError::from)
    }

    pub(super) fn executions(
        &self,
        status: Option<&str>,
        worktree: Option<&str>,
    ) -> Result<Vec<WorkflowExecutionSummaryDto>, ApiRequestError> {
        let mut query = Vec::new();
        if let Some(status) = status {
            query.push(("status", status));
        }
        if let Some(worktree) = worktree {
            query.push(("worktree", worktree));
        }
        self.transport
            .get_json(&["v1", "workflow", "executions"], &query)
            .map_err(ApiRequestError::from)
    }

    fn execution_page(
        &self,
        status: Option<&str>,
        worktree: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkflowExecutionSummaryDto>, ApiRequestError> {
        let limit = limit.to_string();
        let offset = offset.to_string();
        let mut query = Vec::new();
        if let Some(status) = status {
            query.push(("status", status));
        }
        if let Some(worktree) = worktree {
            query.push(("worktree", worktree));
        }
        query.push(("limit", limit.as_str()));
        query.push(("offset", offset.as_str()));
        self.transport
            .get_json(&["v1", "workflow", "executions"], &query)
            .map_err(ApiRequestError::from)
    }

    pub(super) fn start_workflow(
        &self,
        request: &StartExecutionRequest,
    ) -> Result<StartExecutionResponse, ApiRequestError> {
        match self.transport.post_workflow_start(request) {
            Ok(response) => Ok(response),
            Err(
                error @ (LocalApiClientError::Request(_)
                | LocalApiClientError::HttpStatus { status: 409, .. }),
            ) => self
                .reconcile_started_execution(request)
                .map(|execution_id| StartExecutionResponse { execution_id })
                .ok_or_else(|| ApiRequestError::from(error)),
            Err(error) => Err(ApiRequestError::from(error)),
        }
    }

    fn reconcile_started_execution(&self, request: &StartExecutionRequest) -> Option<String> {
        let mut offset = 0;
        let mut matched_execution_id = None;
        loop {
            let executions = self
                .execution_page(
                    Some("active"),
                    Some(request.worktree_path.as_str()),
                    RECONCILIATION_PAGE_LIMIT,
                    offset,
                )
                .ok()?;
            let page_len = executions.len();
            for execution in executions {
                if execution.workflow_name == request.workflow_name
                    && paths_refer_to_same_worktree(
                        &execution.worktree_path,
                        request.worktree_path.as_str(),
                    )
                {
                    if matched_execution_id.is_some() {
                        return None;
                    }
                    matched_execution_id = Some(execution.execution_id);
                }
            }
            if page_len < RECONCILIATION_PAGE_LIMIT {
                return matched_execution_id;
            }
            offset = offset.checked_add(RECONCILIATION_PAGE_LIMIT)?;
        }
    }

    pub(super) fn execution_status(
        &self,
        execution_id: &str,
    ) -> Result<crate::adaptor::protocol::workflow::WorkflowExecutionView, ApiRequestError> {
        self.transport
            .get_json(&["v1", "workflow", "executions", execution_id], &[])
            .map_err(ApiRequestError::from)
    }

    pub(super) fn execution_log(
        &self,
        execution_id: &str,
    ) -> Result<Vec<serde_json::Value>, ApiRequestError> {
        self.transport
            .get_json(&["v1", "workflow", "executions", execution_id, "log"], &[])
            .map_err(ApiRequestError::from)
    }

    pub(super) fn approve(
        &self,
        execution_id: &str,
        request: &ApproveNodeRequest,
    ) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_json(
                &["v1", "workflow", "executions", execution_id, "approve"],
                request,
            )
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn abort(&self, execution_id: &str) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_empty(&["v1", "workflow", "executions", execution_id, "abort"])
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn stop(&self, execution_id: &str) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_empty(&["v1", "workflow", "executions", execution_id, "stop"])
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn resume(&self, execution_id: &str) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_empty(&["v1", "workflow", "executions", execution_id, "resume"])
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn submit_output(
        &self,
        execution_id: &str,
        request: &SubmitArtifactRequest,
    ) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_json(
                &["v1", "workflow", "executions", execution_id, "artifacts"],
                request,
            )
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn validate_output(
        &self,
        execution_id: &str,
        request: &ValidateArtifactRequest,
    ) -> Result<WorkflowValidateOutputResult, ApiRequestError> {
        let response: ValidateArtifactResponse = self
            .transport
            .post_json(
                &[
                    "v1",
                    "workflow",
                    "executions",
                    execution_id,
                    "artifacts:validate",
                ],
                request,
            )
            .map_err(ApiRequestError::from)?;
        Ok(response.into())
    }

    pub(super) fn get_output(
        &self,
        execution_id: &str,
        node: &str,
    ) -> Result<WorkflowGetOutputResult, ApiRequestError> {
        let response: GetArtifactResponse = self
            .transport
            .get_json(
                &[
                    "v1",
                    "workflow",
                    "executions",
                    execution_id,
                    "artifacts",
                    node,
                ],
                &[],
            )
            .map_err(ApiRequestError::from)?;
        Ok(response.into())
    }
}

fn paths_refer_to_same_worktree(left: &str, right: &str) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

pub(super) fn read_with_fallback<T>(
    data_dir: &Path,
    api_request: impl FnOnce(&LocalApiClient) -> Result<T, ApiRequestError>,
    fallback: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let Some(client) = LocalApiClient::discover(data_dir)? else {
        return fallback();
    };
    match api_request(&client) {
        Ok(value) => Ok(value),
        Err(ApiRequestError::Unavailable) => fallback(),
        Err(ApiRequestError::Cli(error)) => Err(error),
    }
}

pub(super) fn mutation<T>(
    data_dir: &Path,
    api_request: impl FnOnce(&LocalApiClient) -> Result<T, ApiRequestError>,
) -> Result<T, CliError> {
    let Some(client) = LocalApiClient::discover(data_dir)? else {
        return Err(app_must_be_running_error());
    };
    match api_request(&client) {
        Ok(value) => Ok(value),
        Err(ApiRequestError::Unavailable) => Err(app_must_be_running_error()),
        Err(ApiRequestError::Cli(error)) => Err(error),
    }
}

/// CLI の明示 target を優先し、workflow session に注入された値を既定値にする。
pub(crate) fn resolve_node_execution_id(explicit: Option<String>) -> Option<String> {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var(NODE_EXECUTION_ID_ENV).ok())
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn start_created_from() -> &'static str {
    if resolve_node_execution_id(None).is_some() {
        "agent"
    } else {
        "cli"
    }
}

fn app_must_be_running_error() -> CliError {
    CliError::Other("この操作には Releash アプリの起動が必要です".to_string())
}

fn api_error(status: u16, message: Option<&str>) -> CliError {
    let message = message
        .filter(|message| !message.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("local API error ({status})"));
    match status {
        404 => CliError::NotFound(message),
        400 | 409 | 422 => CliError::InvalidInput(message),
        401 => CliError::Other(format!("local API の認証に失敗しました: {message}")),
        _ => CliError::Other(message),
    }
}

fn ensure_mutation_ok(response: MutationResponse) -> Result<(), ApiRequestError> {
    if response.ok {
        Ok(())
    } else {
        Err(ApiRequestError::Cli(CliError::Other(
            "local API mutation response が不正です: ok=false".to_string(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use tempfile::TempDir;

    use super::*;
    use crate::adaptor::controller::api::{self, test_support as api_test_support};
    use crate::adaptor::controller::command::workflow::runtime::{
        approve_workflow_node_with_runtime, ApproveWorkflowNodeArgs,
    };
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind, WorkflowDefinitionYaml,
    };
    use crate::adaptor::gateway::workflow::storage;
    use crate::cli::{file_direct, output, workflow};
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatusFilter};
    use crate::infrastructure::local_api::local_api_discovery_path;
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};
    use crate::usecase::workflow::command::ApprovalCommand;

    fn write_discovery(data_dir: &Path, port: u16, token: &str) {
        std::fs::write(
            local_api_discovery_path(data_dir),
            serde_json::json!({"port": port, "token": token, "pid": 42}).to_string(),
        )
        .unwrap();
    }

    fn command_workflow(name: &str) -> WorkflowDefinitionYaml {
        WorkflowDefinitionYaml {
            name: name.to_string(),
            description: "live local API boundary fixture".to_string(),
            nodes: vec![NodeDefinition {
                name: "execute".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..NodeDefinition::default()
            }],
            ..WorkflowDefinitionYaml::default()
        }
    }

    #[tokio::test]
    async fn ui_cli_and_agent_approval_adapters_share_one_typed_command_boundary() {
        let data = TempDir::new().unwrap();
        let execution_id = "00000000-0000-4000-8000-000000000321";
        let (router, runtime, gateway) = api_test_support::test_router(data.path(), "secret");

        approve_workflow_node_with_runtime(
            runtime.as_ref(),
            ApproveWorkflowNodeArgs {
                execution_id: execution_id.to_string(),
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-shared".to_string()),
                comment: Some("approved by boundary".to_string()),
            },
        )
        .await
        .unwrap();

        let explicit_cli_request = ApproveNodeRequest {
            node: "review".to_string(),
            node_execution_id: Some("node-execution-shared".to_string()),
            comment: Some("approved by boundary".to_string()),
        };
        let explicit_cli = api_test_support::send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/approve"),
            serde_json::to_value(explicit_cli_request).unwrap(),
        )
        .await;
        assert_eq!(explicit_cli.0, StatusCode::OK);

        let agent_node_execution_id = {
            let _lock = TEST_ENV_LOCK.lock().unwrap();
            let _node_execution =
                EnvVarGuard::set_value(NODE_EXECUTION_ID_ENV, "node-execution-shared");
            resolve_node_execution_id(None)
        };
        let agent_request = ApproveNodeRequest {
            node: "review".to_string(),
            node_execution_id: agent_node_execution_id,
            comment: Some("approved by boundary".to_string()),
        };
        let agent = api_test_support::send_json(
            &router,
            &format!("/v1/workflow/executions/{execution_id}/approve"),
            serde_json::to_value(agent_request).unwrap(),
        )
        .await;
        assert_eq!(agent.0, StatusCode::OK);

        let expected = ApprovalCommand {
            execution_id: execution_id.to_string(),
            node_name: "review".to_string(),
            node_execution_id: Some("node-execution-shared".to_string()),
            comment: Some("approved by boundary".to_string()),
        };
        assert_eq!(
            gateway.commands.lock().unwrap().approvals,
            vec![expected.clone(), expected.clone(), expected]
        );
    }

    #[test]
    fn cli_commands_cross_discovery_and_live_http_into_shared_typed_usecases() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _node_execution =
            EnvVarGuard::set_value(NODE_EXECUTION_ID_ENV, "node-execution-shared");
        let client_data = TempDir::new().unwrap();
        let query_data = TempDir::new().unwrap();
        let workflows = TempDir::new().unwrap();
        let execution_id = "00000000-0000-4000-8000-000000000321";

        storage::save_workflow(workflows.path(), &command_workflow("declared-name")).unwrap();
        storage::save_workflow(workflows.path(), &command_workflow("review")).unwrap();
        std::fs::rename(
            workflows.path().join("declared-name.yml"),
            workflows.path().join("different-filename.yml"),
        )
        .unwrap();
        api_test_support::seed_query_execution(query_data.path(), execution_id);
        api_test_support::seed_submitted_output(query_data.path(), execution_id);

        let (_workflow_usecase, runtime, gateway) = api_test_support::usecases(query_data.path());
        gateway.resolve_workflows_from(
            workflows.path().to_path_buf(),
            workflows.path().to_path_buf(),
        );
        let binding = match crate::infrastructure::local_api::LocalApiServerBinding::bind(
            client_data.path().to_path_buf(),
        ) {
            Ok(binding) => binding,
            Err(error)
                if error.to_string().contains("Operation not permitted")
                    || error.to_string().contains("Permission denied") =>
            {
                eprintln!(
                    "skipping live loopback local API test because this sandbox forbids bind: {error}"
                );
                return;
            }
            Err(error) => panic!("the live local API must bind to loopback: {error}"),
        };
        let router = api::build_router(
            Arc::new(
                crate::adaptor::controller::wiring::build_file_direct_workflow_read_usecase(
                    query_data.path().to_path_buf(),
                    Some(workflows.path().to_path_buf()),
                )
                .unwrap(),
            ),
            runtime.clone(),
            binding.bearer_token(),
        );
        let server_runtime = tokio::runtime::Runtime::new().unwrap();
        let server = binding.start(router, server_runtime.handle());
        assert!(local_api_discovery_path(client_data.path()).is_file());
        assert!(!client_data.path().join("workflow_executions").exists());

        let listed: Vec<WorkflowSummaryDto> = serde_json::from_str(
            &workflow::cmd_list(workflows.path(), client_data.path(), true).unwrap(),
        )
        .unwrap();
        assert_eq!(
            listed,
            file_direct::list_workflows(workflows.path(), query_data.path()).unwrap()
        );
        assert!(listed
            .iter()
            .any(|summary| summary.name == "review" && summary.is_running));

        let started = workflow::cmd_start(
            client_data.path(),
            "declared-name".to_string(),
            Some("review this through the CLI".to_string()),
            Some(PathBuf::from("/repo")),
            Some("edit".to_string()),
        )
        .unwrap();
        assert_eq!(
            started,
            "started: execution_id=00000000-0000-4000-8000-000000000001\n"
        );

        let executions: Vec<WorkflowExecutionSummaryDto> = serde_json::from_str(
            &workflow::cmd_executions(client_data.path(), Some("active".to_string()), None, true)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            executions,
            file_direct::list_executions(
                query_data.path(),
                Some(ExecutionStatusFilter::Active),
                None,
            )
            .unwrap()
        );
        assert_eq!(executions[0].execution_id, execution_id);
        assert_eq!(executions[0].workflow_name, "review");

        let status: serde_json::Value = serde_json::from_str(
            &workflow::cmd_status(client_data.path(), execution_id, true).unwrap(),
        )
        .unwrap();
        assert_eq!(status["id"], execution_id);
        assert_eq!(status["artifacts"][0]["nodeName"], "request");

        let logs: serde_json::Value = serde_json::from_str(
            &workflow::cmd_logs(client_data.path(), execution_id, true).unwrap(),
        )
        .unwrap();
        assert_eq!(logs[0]["event"], "execution_started");
        assert_eq!(logs[0]["request"], "review this change");
        let missing_execution_id = "00000000-0000-4000-8000-000000000999";
        let missing_logs =
            workflow::cmd_logs(client_data.path(), missing_execution_id, true).unwrap_err();
        assert!(
            matches!(missing_logs, CliError::NotFound(message) if message.contains(missing_execution_id))
        );

        server_runtime
            .block_on(approve_workflow_node_with_runtime(
                runtime.as_ref(),
                ApproveWorkflowNodeArgs {
                    execution_id: execution_id.to_string(),
                    node_name: "review".to_string(),
                    node_execution_id: Some("node-execution-shared".to_string()),
                    comment: Some("approved by boundary".to_string()),
                },
            ))
            .unwrap();
        workflow::cmd_approve(
            client_data.path(),
            execution_id,
            "review".to_string(),
            Some("node-execution-shared".to_string()),
            Some("approved by boundary".to_string()),
        )
        .unwrap();
        workflow::cmd_approve(
            client_data.path(),
            execution_id,
            "review".to_string(),
            None,
            Some("approved by boundary".to_string()),
        )
        .unwrap();
        assert_eq!(
            workflow::cmd_stop(client_data.path(), execution_id).unwrap(),
            format!("stopped: execution_id={execution_id}\n")
        );
        assert_eq!(
            workflow::cmd_resume(client_data.path(), execution_id).unwrap(),
            format!("resumed: execution_id={execution_id}\n")
        );
        workflow::cmd_abort(client_data.path(), execution_id).unwrap();

        output::cmd_output_submit(
            client_data.path(),
            execution_id,
            "review",
            None,
            "review-result",
            Some(r#"{"status":"approved"}"#.to_string()),
            None,
        )
        .unwrap();
        let artifact_file = client_data.path().join("artifact.json");
        std::fs::write(&artifact_file, r#"{"status":"approved"}"#).unwrap();
        assert_eq!(
            output::cmd_output_validate(
                client_data.path(),
                execution_id,
                "review",
                "review-result",
                &artifact_file,
            )
            .unwrap(),
            "ok: artifact schema 'review-result' is satisfied\n"
        );
        let output: serde_json::Value = serde_json::from_str(
            &output::cmd_output_get(client_data.path(), execution_id, "review", true).unwrap(),
        )
        .unwrap();
        assert_eq!(output["status"], "submitted");
        assert_eq!(output["contract"], "review-result");
        assert_eq!(
            output["artifact"],
            serde_json::json!({"status": "approved"})
        );

        {
            let commands = gateway.commands.lock().unwrap();
            assert_eq!(commands.starts.len(), 1);
            assert_eq!(commands.starts[0].workflow_name, "declared-name");
            assert_eq!(commands.starts[0].workflow.name, "declared-name");
            assert_eq!(commands.starts[0].worktree_path, "/repo");
            assert_eq!(
                commands.starts[0].request.as_deref(),
                Some("review this through the CLI")
            );
            assert_eq!(commands.starts[0].created_from, ExecutionOrigin::Agent);
            assert_eq!(commands.starts[0].permission_mode, "edit");

            let expected_approval = ApprovalCommand {
                execution_id: execution_id.to_string(),
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-shared".to_string()),
                comment: Some("approved by boundary".to_string()),
            };
            assert_eq!(
                commands.approvals,
                vec![
                    expected_approval.clone(),
                    expected_approval.clone(),
                    expected_approval,
                ]
            );
            assert_eq!(commands.aborts.len(), 1);
            assert_eq!(commands.aborts[0].execution_id, execution_id);
            assert_eq!(commands.stops.len(), 1);
            assert_eq!(commands.stops[0].execution_id, execution_id);
            assert_eq!(commands.resumes.len(), 1);
            assert_eq!(commands.resumes[0].execution_id, execution_id);
            assert_eq!(commands.outputs.len(), 1);
            assert_eq!(commands.outputs[0].execution_id, execution_id);
            assert_eq!(commands.outputs[0].node_name, "review");
            assert_eq!(
                commands.outputs[0].node_execution_id.as_deref(),
                Some("node-execution-shared")
            );
            assert_eq!(commands.outputs[0].contract, "review-result");
            assert_eq!(
                commands.outputs[0].artifact,
                serde_json::json!({"status": "approved"})
            );
        }

        std::fs::copy(
            workflows.path().join("different-filename.yml"),
            workflows.path().join("duplicate-name.yml"),
        )
        .unwrap();
        let duplicate = workflow::cmd_start(
            client_data.path(),
            "declared-name".to_string(),
            None,
            Some(PathBuf::from("/repo")),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(duplicate, CliError::InvalidInput(message) if message.contains("WFS006") && message.contains("declared-name"))
        );

        assert_eq!(gateway.commands.lock().unwrap().starts.len(), 1);

        server.shutdown();
        assert!(!local_api_discovery_path(client_data.path()).exists());
    }

    #[test]
    fn discovery_missing_uses_read_fallback() {
        let temp = TempDir::new().unwrap();
        let value = read_with_fallback(temp.path(), |_| unreachable!(), || Ok(42)).unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    fn discovery_missing_rejects_mutation_in_japanese() {
        let temp = TempDir::new().unwrap();
        let result: Result<(), CliError> = mutation(temp.path(), |_| unreachable!());
        let error = result.unwrap_err();
        assert!(
            matches!(error, CliError::Other(message) if message.contains("アプリの起動が必要"))
        );
    }

    fn assert_start_reconciles_committed_execution(first_response: Option<&'static [u8]>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worktree = TempDir::new().unwrap();
        let worktree_path = worktree.path().to_string_lossy().into_owned();
        let response_worktree = worktree_path.clone();
        let execution_id = "00000000-0000-4000-8000-000000000777";
        let server = std::thread::spawn(move || {
            for request_index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                }
                if request_index == 0 {
                    if let Some(response) = first_response {
                        stream.write_all(response).unwrap();
                    }
                    continue;
                }
                let request = String::from_utf8_lossy(&bytes);
                let request_target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap();
                let request_url =
                    url::Url::parse(&format!("http://localhost{request_target}")).unwrap();
                assert_eq!(request_url.path(), "/v1/workflow/executions");
                let query = request_url
                    .query_pairs()
                    .collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(
                    query.get("status").map(|value| value.as_ref()),
                    Some("active")
                );
                assert_eq!(
                    query.get("worktree").map(|value| value.as_ref()),
                    Some(response_worktree.as_str())
                );
                assert_eq!(query.get("limit").map(|value| value.as_ref()), Some("100"));
                let expected_offset = if request_index == 1 { "0" } else { "100" };
                assert_eq!(
                    query.get("offset").map(|value| value.as_ref()),
                    Some(expected_offset)
                );
                let body = if request_index == 1 {
                    (0..RECONCILIATION_PAGE_LIMIT)
                        .map(|index| {
                            serde_json::json!({
                                "executionId": format!("00000000-0000-4000-8000-{index:012}"),
                                "workflowName": "another-workflow",
                                "status": "running",
                                "worktreePath": response_worktree,
                                "currentNode": "review",
                                "createdFrom": "cli",
                                "startedAt": 1.0,
                                "updatedAt": 1.0,
                                "totalTokenUsage": {"inputTokens": 0, "outputTokens": 0}
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![serde_json::json!({
                        "executionId": execution_id,
                        "workflowName": "slow-start",
                        "status": "running",
                        "worktreePath": response_worktree,
                        "currentNode": "review",
                        "createdFrom": "cli",
                        "startedAt": 1.0,
                        "updatedAt": 1.0,
                        "totalTokenUsage": {"inputTokens": 0, "outputTokens": 0}
                    })]
                };
                let body = serde_json::to_string(&body).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let data = TempDir::new().unwrap();
        write_discovery(data.path(), port, "secret");
        let client = LocalApiClient::discover(data.path()).unwrap().unwrap();

        let response = client
            .start_workflow(&StartExecutionRequest {
                workflow_name: "slow-start".to_string(),
                worktree_path,
                request: String::new(),
                permission_mode: None,
                created_from: Some("cli".to_string()),
            })
            .unwrap();

        assert_eq!(response.execution_id, execution_id);
        server.join().unwrap();
    }

    #[test]
    fn start_reconciles_the_committed_execution_after_an_unknown_transport_result() {
        // The server committed the start but the connection disappeared before the response
        // reached the CLI, reproducing the unknown-result boundary.
        assert_start_reconciles_committed_execution(None);
    }

    #[test]
    fn repeated_start_recovers_the_existing_execution_from_worktree_conflict() {
        assert_start_reconciles_committed_execution(Some(
            b"HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: 51\r\nConnection: close\r\n\r\n{\"code\":\"invalid_state\",\"message\":\"already active\"}",
        ));
    }

    #[test]
    fn unavailable_api_uses_read_fallback() {
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), 43123, "secret");
        let value = read_with_fallback(
            temp.path(),
            |_| Err(ApiRequestError::Unavailable),
            || {
                Ok(vec![WorkflowSummaryDto {
                    name: "fallback".to_string(),
                    description: String::new(),
                    builtin: false,
                    is_running: false,
                }])
            },
        )
        .unwrap();
        assert_eq!(value[0].name, "fallback");
    }

    #[test]
    fn unauthorized_response_does_not_use_fallback() {
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), 43123, "secret");
        let result: Result<Vec<WorkflowSummaryDto>, CliError> = read_with_fallback(
            temp.path(),
            |_| {
                Err(ApiRequestError::Cli(api_error(
                    StatusCode::UNAUTHORIZED.as_u16(),
                    Some("bad token"),
                )))
            },
            || panic!("401 must not fall back"),
        );
        let error = result.unwrap_err();
        assert!(matches!(error, CliError::Other(message) if message.contains("認証に失敗")));
    }

    #[test]
    fn malformed_discovery_does_not_use_fallback() {
        let temp = TempDir::new().unwrap();
        std::fs::write(local_api_discovery_path(temp.path()), "not-json").unwrap();
        let result: Result<Vec<WorkflowSummaryDto>, CliError> = read_with_fallback(
            temp.path(),
            |_| unreachable!(),
            || panic!("malformed discovery must not fall back"),
        );
        let error = result.unwrap_err();
        assert!(
            matches!(error, CliError::Other(message) if message.contains("discovery file が不正"))
        );
    }

    #[test]
    fn node_execution_id_prefers_explicit_value_and_falls_back_to_environment() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let _guard = EnvVarGuard::set_value(NODE_EXECUTION_ID_ENV, "node-execution-env");

        assert_eq!(
            resolve_node_execution_id(Some("node-execution-explicit".to_string())),
            Some("node-execution-explicit".to_string())
        );
        assert_eq!(
            resolve_node_execution_id(None),
            Some("node-execution-env".to_string())
        );
        assert_eq!(start_created_from(), "agent");
    }
}
