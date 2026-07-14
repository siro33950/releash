use std::path::Path;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use super::common::CliError;
use crate::adaptor::controller::api::protocol::{
    ApproveNodeRequest, GetArtifactResponse, MutationResponse, StartExecutionRequest,
    StartExecutionResponse, SubmitArtifactRequest, ValidateArtifactRequest,
    ValidateArtifactResponse,
};
use crate::infrastructure::local_api::{local_api_discovery_path, LocalApiDiscovery};
use crate::usecase::workflow::dto::{WorkflowExecutionSummaryDto, WorkflowSummaryDto};
use crate::usecase::workflow::{WorkflowGetOutputResult, WorkflowValidateOutputResult};

const NODE_EXECUTION_ID_ENV: &str = "RELEASH_NODE_EXECUTION_ID";

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

#[derive(Debug, Clone)]
pub(super) struct LocalApiClient {
    base_url: Url,
    token: String,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    #[allow(dead_code)]
    code: String,
    message: String,
}

impl LocalApiClient {
    fn discover(data_dir: &Path) -> Result<Option<Self>, CliError> {
        let path = local_api_discovery_path(data_dir);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CliError::Other(format!(
                    "local API discovery file の読み込みに失敗しました ({}): {error}",
                    path.display()
                )));
            }
        };
        let discovery: LocalApiDiscovery = serde_json::from_str(&contents).map_err(|error| {
            CliError::Other(format!(
                "local API discovery file が不正です ({}): {error}",
                path.display()
            ))
        })?;
        if discovery.port == 0 || discovery.token.trim().is_empty() {
            return Err(CliError::Other(format!(
                "local API discovery file が不正です ({}): port と token が必要です",
                path.display()
            )));
        }
        let base_url = Url::parse(&format!("http://127.0.0.1:{}/", discovery.port))
            .map_err(|error| CliError::Other(format!("local API URL が不正です: {error}")))?;
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| {
                CliError::Other(format!("local API client の初期化に失敗しました: {error}"))
            })?;
        Ok(Some(Self {
            base_url,
            token: discovery.token,
            client,
        }))
    }

    pub(super) fn workflows(&self) -> Result<Vec<WorkflowSummaryDto>, ApiRequestError> {
        self.get(self.endpoint(&["v1", "workflows"])?)
    }

    pub(super) fn executions(
        &self,
        status: Option<&str>,
        worktree: Option<&str>,
    ) -> Result<Vec<WorkflowExecutionSummaryDto>, ApiRequestError> {
        let mut url = self.endpoint(&["v1", "workflow", "executions"])?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(status) = status {
                query.append_pair("status", status);
            }
            if let Some(worktree) = worktree {
                query.append_pair("worktree", worktree);
            }
        }
        self.get(url)
    }

    pub(super) fn start_workflow(
        &self,
        request: &StartExecutionRequest,
    ) -> Result<StartExecutionResponse, ApiRequestError> {
        self.post_json(self.endpoint(&["v1", "workflow", "executions"])?, request)
    }

    pub(super) fn execution_status(
        &self,
        execution_id: &str,
    ) -> Result<crate::adaptor::protocol::workflow::WorkflowExecutionView, ApiRequestError> {
        self.get(self.endpoint(&["v1", "workflow", "executions", execution_id])?)
    }

    pub(super) fn execution_log(
        &self,
        execution_id: &str,
    ) -> Result<Vec<serde_json::Value>, ApiRequestError> {
        self.get(self.endpoint(&["v1", "workflow", "executions", execution_id, "log"])?)
    }

    pub(super) fn approve(
        &self,
        execution_id: &str,
        request: &ApproveNodeRequest,
    ) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self.post_json(
            self.endpoint(&["v1", "workflow", "executions", execution_id, "approve"])?,
            request,
        )?;
        ensure_mutation_ok(response)
    }

    pub(super) fn abort(&self, execution_id: &str) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self.post_empty(self.endpoint(&[
            "v1",
            "workflow",
            "executions",
            execution_id,
            "abort",
        ])?)?;
        ensure_mutation_ok(response)
    }

    pub(super) fn submit_output(
        &self,
        execution_id: &str,
        request: &SubmitArtifactRequest,
    ) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self.post_json(
            self.endpoint(&["v1", "workflow", "executions", execution_id, "artifacts"])?,
            request,
        )?;
        ensure_mutation_ok(response)
    }

    pub(super) fn validate_output(
        &self,
        execution_id: &str,
        request: &ValidateArtifactRequest,
    ) -> Result<WorkflowValidateOutputResult, ApiRequestError> {
        let response: ValidateArtifactResponse = self.post_json(
            self.endpoint(&[
                "v1",
                "workflow",
                "executions",
                execution_id,
                "artifacts:validate",
            ])?,
            request,
        )?;
        Ok(response.into())
    }

    pub(super) fn get_output(
        &self,
        execution_id: &str,
        node: &str,
    ) -> Result<WorkflowGetOutputResult, ApiRequestError> {
        let response: GetArtifactResponse = self.get(self.endpoint(&[
            "v1",
            "workflow",
            "executions",
            execution_id,
            "artifacts",
            node,
        ])?)?;
        Ok(response.into())
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, ApiRequestError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| {
                ApiRequestError::Cli(CliError::Other(
                    "local API URL を構築できません".to_string(),
                ))
            })?
            .extend(segments);
        Ok(url)
    }

    fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T, ApiRequestError> {
        self.send(self.client.get(url))
    }

    fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        url: Url,
        body: &B,
    ) -> Result<T, ApiRequestError> {
        self.send(self.client.post(url).json(body))
    }

    fn post_empty<T: DeserializeOwned>(&self, url: Url) -> Result<T, ApiRequestError> {
        self.send(self.client.post(url))
    }

    fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, ApiRequestError> {
        let response = self
            .authenticated(request)
            .send()
            .map_err(classify_transport_error)?;
        let status = response.status();
        let bytes = response.bytes().map_err(classify_transport_error)?;
        if !status.is_success() {
            return Err(ApiRequestError::Cli(api_error(status, &bytes)));
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            ApiRequestError::Cli(CliError::Other(format!(
                "local API response が不正です: {error}"
            )))
        })
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.token)
    }
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

fn classify_transport_error(error: reqwest::Error) -> ApiRequestError {
    if error.is_connect() {
        ApiRequestError::Unavailable
    } else {
        ApiRequestError::Cli(CliError::Other(format!(
            "local API request に失敗しました: {error}"
        )))
    }
}

fn api_error(status: StatusCode, body: &[u8]) -> CliError {
    let parsed = serde_json::from_slice::<ErrorResponse>(body).ok();
    let message = parsed
        .map(|error| error.message)
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| format!("local API error ({status})"));
    match status {
        StatusCode::NOT_FOUND => CliError::NotFound(message),
        StatusCode::BAD_REQUEST | StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY => {
            CliError::InvalidInput(message)
        }
        StatusCode::UNAUTHORIZED => {
            CliError::Other(format!("local API の認証に失敗しました: {message}"))
        }
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
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;
    use crate::adaptor::controller::api::{self, test_support as api_test_support};
    use crate::adaptor::controller::command::workflow::runtime::{
        approve_workflow_node_with_runtime, ApproveWorkflowNodeArgs,
    };
    use crate::adaptor::gateway::workflow::schema::{
        CommandSpec, NodeDefinition, NodeKind, Workflow,
    };
    use crate::adaptor::gateway::workflow::storage;
    use crate::cli::{file_direct, output, workflow};
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatusFilter};
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};
    use crate::usecase::workflow::command::ApprovalCommand;

    fn write_discovery(data_dir: &Path, port: u16, token: &str) {
        std::fs::write(
            local_api_discovery_path(data_dir),
            serde_json::json!({"port": port, "token": token, "pid": 42}).to_string(),
        )
        .unwrap();
    }

    fn command_workflow(name: &str) -> Workflow {
        Workflow {
            name: name.to_string(),
            description: "live local API boundary fixture".to_string(),
            nodes: vec![NodeDefinition {
                name: "execute".to_string(),
                kind: NodeKind::Command(CommandSpec {
                    command: "true".to_string(),
                }),
                ..NodeDefinition::default()
            }],
            ..Workflow::default()
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
                if error.contains("Operation not permitted")
                    || error.contains("Permission denied") =>
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

        let discovery: LocalApiDiscovery = serde_json::from_str(
            &std::fs::read_to_string(local_api_discovery_path(client_data.path())).unwrap(),
        )
        .unwrap();
        let duplicate_response = reqwest::blocking::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/v1/workflow/executions",
                discovery.port
            ))
            .bearer_auth(discovery.token)
            .json(&serde_json::json!({
                "workflow_name": "declared-name",
                "worktree_path": "/repo",
                "request": "",
                "permission_mode": "ask",
                "created_from": "agent"
            }))
            .send()
            .unwrap();
        assert_eq!(duplicate_response.status(), StatusCode::BAD_REQUEST);
        let duplicate_body: serde_json::Value = duplicate_response.json().unwrap();
        assert_eq!(duplicate_body["code"], "validation_error");
        assert!(duplicate_body["message"]
            .as_str()
            .unwrap()
            .contains("WFS006"));
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
                    StatusCode::UNAUTHORIZED,
                    br#"{"code":"unauthorized","message":"bad token"}"#,
                )))
            },
            || panic!("401 must not fall back"),
        );
        let error = result.unwrap_err();
        assert!(matches!(error, CliError::Other(message) if message.contains("認証に失敗")));
    }

    #[test]
    fn every_request_builder_receives_bearer_token() {
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), 43123, "secret");
        let client = LocalApiClient::discover(temp.path()).unwrap().unwrap();
        let request = client
            .authenticated(client.client.get(client.base_url.clone()))
            .build()
            .unwrap();
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .unwrap(),
            "Bearer secret"
        );
    }

    #[test]
    fn local_api_client_ignores_environment_proxy_and_connects_to_loopback() {
        let _lock = TEST_ENV_LOCK.lock().unwrap();
        let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let proxy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let _http_proxy = EnvVarGuard::set_value("HTTP_PROXY", &proxy_url);
        let _http_proxy_lower = EnvVarGuard::set_value("http_proxy", &proxy_url);
        let _all_proxy = EnvVarGuard::set_value("ALL_PROXY", &proxy_url);
        let _all_proxy_lower = EnvVarGuard::set_value("all_proxy", &proxy_url);
        let _no_proxy = EnvVarGuard::set_value("NO_PROXY", "");
        let _no_proxy_lower = EnvVarGuard::set_value("no_proxy", "");
        let target_port = target.local_addr().unwrap().port();
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_server = received.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        request.extend_from_slice(&buffer[..read]);
                        if request
                            .windows(b"proxy-secret-marker".len())
                            .any(|window| window == b"proxy-secret-marker")
                        {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(error) => panic!("failed to read direct request: {error}"),
                }
            }
            *received_for_server.lock().unwrap() = request;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 55\r\nConnection: close\r\n\r\n{\"execution_id\":\"00000000-0000-4000-8000-000000000001\"}",
                )
                .unwrap();
        });
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), target_port, "proxy-secret-token");
        let client = LocalApiClient::discover(temp.path()).unwrap().unwrap();
        let response = client
            .start_workflow(&StartExecutionRequest {
                workflow_name: "proxy-secret-marker".to_string(),
                worktree_path: "/repo".to_string(),
                request: "proxy-secret-body".to_string(),
                permission_mode: None,
                created_from: Some("cli".to_string()),
            })
            .unwrap();
        assert_eq!(
            response.execution_id,
            "00000000-0000-4000-8000-000000000001"
        );
        server.join().unwrap();
        let direct_request = String::from_utf8_lossy(&received.lock().unwrap()).into_owned();
        assert!(direct_request
            .to_ascii_lowercase()
            .contains("authorization: bearer proxy-secret-token"));
        assert!(direct_request.contains("proxy-secret-body"));
        assert!(
            matches!(proxy.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn response_body_timeout_is_an_api_error_and_does_not_fall_back() {
        let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let target_port = target.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n[",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), target_port, "secret");
        let fallback_called = std::cell::Cell::new(false);
        let result: Result<Vec<WorkflowSummaryDto>, CliError> = read_with_fallback(
            temp.path(),
            |discovered| {
                let short_timeout = LocalApiClient {
                    base_url: discovered.base_url.clone(),
                    token: discovered.token.clone(),
                    client: Client::builder()
                        .no_proxy()
                        .connect_timeout(Duration::from_millis(100))
                        .timeout(Duration::from_millis(100))
                        .build()
                        .unwrap(),
                };
                short_timeout.workflows()
            },
            || {
                fallback_called.set(true);
                Ok(Vec::new())
            },
        );
        let error = result.unwrap_err();
        assert!(matches!(error, CliError::Other(message) if message.contains("local API request")));
        assert!(!fallback_called.get());
        server.join().unwrap();
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
