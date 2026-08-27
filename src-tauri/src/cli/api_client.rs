use std::path::Path;

use super::common::CliError;
use crate::adaptor::controller::api::protocol::{
    GetArtifactResponse, MutationResponse, SubmitOutputRequest,
};
use crate::adaptor::gateway::local_api::{LocalApiClientError, LocalApiClientGateway};
use crate::adaptor::protocol::provider_lifecycle::{
    ProviderLifecycleReceiveRequest, ProviderLifecycleReceiveResponse,
};
use crate::usecase::workflow::WorkflowGetOutputResult;

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
    transport: LocalApiClientGateway,
}

impl LocalApiClient {
    fn discover(data_dir: &Path) -> Result<Option<Self>, CliError> {
        LocalApiClientGateway::discover(data_dir)
            .map(|client| client.map(|transport| Self { transport }))
            .map_err(discovery_error)
    }

    pub(super) fn execution_status(
        &self,
        execution_id: &str,
    ) -> Result<crate::adaptor::protocol::workflow::WorkflowExecutionView, ApiRequestError> {
        self.transport
            .get_json(&["v1", "workflow", "executions", execution_id], &[])
            .map_err(ApiRequestError::from)
    }

    pub(super) fn submit_output(
        &self,
        node_execution_id: &str,
        request: &SubmitOutputRequest,
    ) -> Result<(), ApiRequestError> {
        let response: MutationResponse = self
            .transport
            .post_json(
                &[
                    "v1",
                    "workflow",
                    "node-executions",
                    node_execution_id,
                    "submit",
                ],
                request,
            )
            .map_err(ApiRequestError::from)?;
        ensure_mutation_ok(response)
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

    /// workflow diagnostics を local API 経由で取得する。診断結果 DTO をそのまま返す。
    pub(super) fn workflow_diagnostics(
        &self,
        dir: Option<&str>,
    ) -> Result<serde_json::Value, ApiRequestError> {
        let query: Vec<(&str, &str)> = dir.into_iter().map(|dir| ("dir", dir)).collect();
        self.transport
            .get_json(&["v1", "workflow", "diagnostics"], &query)
            .map_err(ApiRequestError::from)
    }

    pub(super) fn receive_provider_lifecycle(
        &self,
        request: &ProviderLifecycleReceiveRequest,
    ) -> Result<ProviderLifecycleReceiveResponse, ApiRequestError> {
        self.transport
            .post_json(&["v1", "provider-lifecycle", "signals"], request)
            .map_err(ApiRequestError::from)
    }
}

fn discovery_error(error: LocalApiClientError) -> CliError {
    CliError::Other(error.to_string())
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

/// アプリ起動を要する mutation。local API へ到達できない場合は
/// 「アプリ起動が必要」失敗になる。
pub(super) fn mutation<T>(
    data_dir: &Path,
    api_request: impl FnOnce(&LocalApiClient) -> Result<T, ApiRequestError>,
) -> Result<T, CliError> {
    require_running(request_classified(data_dir, api_request))
}

/// アプリ起動を要する read-only query。fallback を持たず、失敗表現は
/// mutation と同じにする。read と mutation を別入口に保つのは、file_direct が
/// 定める「read だけが fallback を持てる」区別を呼び出し側で崩さないためである。
pub(super) fn read_without_fallback<T>(
    data_dir: &Path,
    api_request: impl FnOnce(&LocalApiClient) -> Result<T, ApiRequestError>,
) -> Result<T, CliError> {
    require_running(request_classified(data_dir, api_request))
}

fn require_running<T>(result: Result<T, ApiRequestError>) -> Result<T, CliError> {
    match result {
        Ok(value) => Ok(value),
        Err(ApiRequestError::Unavailable) => Err(app_must_be_running_error()),
        Err(ApiRequestError::Cli(error)) => Err(error),
    }
}

pub(super) fn request_classified<T>(
    data_dir: &Path,
    api_request: impl FnOnce(&LocalApiClient) -> Result<T, ApiRequestError>,
) -> Result<T, ApiRequestError> {
    let client = LocalApiClient::discover(data_dir).map_err(ApiRequestError::Cli)?;
    let Some(client) = client else {
        return Err(ApiRequestError::Unavailable);
    };
    api_request(&client)
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
#[path = "api_client_test.rs"]
mod api_client_tests;
