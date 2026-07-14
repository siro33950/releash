use std::path::Path;
use std::sync::Arc;

use super::common::{validate_execution_id, CliError};
use crate::adaptor::gateway::workflow::{
    PendingWorkflowCommandFileRepository, WorkflowDefinitionFileRepository,
    WorkflowDefinitionFileSourceGateway, WorkflowEventLogRepository,
    WorkflowExecutionFileRepository, WorkflowExecutionProjectionLogRepository,
    WorkflowFacetFileRepository,
};
use crate::domain::workflow::WorkflowExecutionSummary;
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::workflow::command::WorkflowPendingCommandUsecase;
use crate::usecase::workflow::ports::{
    PendingWorkflowCommand, WorkflowEventDraft, WorkflowEventRepository,
};
use crate::usecase::workflow::query_service::WorkflowQueryService;

const NODE_EXECUTION_ID_ENV: &str = "RELEASH_NODE_EXECUTION_ID";

pub(super) use crate::adaptor::gateway::workflow::pending_command::PendingCommandPayload as CliRequestPayload;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingEnqueueOutput {
    pub(super) execution_id: String,
    pub(super) request_id: String,
    pub(super) path: String,
}

impl PendingEnqueueOutput {
    pub(super) fn format_stdout_line(&self) -> String {
        format!(
            "queued: execution_id={} request_id={} ({})",
            self.execution_id, self.request_id, self.path
        )
    }
}

/// CLI の明示 target を優先し、session / command 実行環境に注入された
/// NodeExecution ID を既定値として補う。
pub(super) fn resolve_node_execution_id(explicit: Option<String>) -> Option<String> {
    explicit
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var(NODE_EXECUTION_ID_ENV).ok())
        .filter(|value| !value.trim().is_empty())
}

/// CLI file-direct surface 向けの query service composition root。
///
/// Tauri surface と同じ [`WorkflowQueryService`] と canonical
/// `WorkflowExecution` projection repository を使い、surface ごとの観測ロジックを持たない。
pub(super) fn file_direct_query_service(data_dir: &Path) -> WorkflowQueryService {
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    file_direct_query_service_with_workflows(data_dir, &workflows_dir)
}

pub(super) fn file_direct_query_service_with_workflows(
    data_dir: &Path,
    workflows_dir: &Path,
) -> WorkflowQueryService {
    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.to_path_buf(),
        workflows_dir.to_path_buf(),
    ));
    WorkflowQueryService::new(
        Arc::new(WorkflowExecutionFileRepository::new(data_dir.to_path_buf())),
        definitions,
        Arc::new(WorkflowDefinitionFileSourceGateway::new(
            workflows_dir.to_path_buf(),
            workflows_dir.to_path_buf(),
        )),
        Arc::new(WorkflowFacetFileRepository::new(
            workflows_dir.to_path_buf(),
        )),
        Arc::new(WorkflowEventLogRepository::new(data_dir.to_path_buf())),
        Arc::new(WorkflowExecutionProjectionLogRepository::new(
            data_dir.to_path_buf(),
        )),
    )
}

pub(super) fn enqueue_pending_command(
    data_dir: &Path,
    execution_id: &str,
    payload: CliRequestPayload,
) -> Result<PendingEnqueueOutput, CliError> {
    validate_execution_id(execution_id)?;
    ensure_execution_exists(data_dir, execution_id)?;
    let command_id = uuid::Uuid::new_v4().to_string();
    let requested_at = current_timestamp();
    let payload = serde_json::to_value(payload).map_err(|error| {
        CliError::Other(format!("Failed to serialize pending command: {error}"))
    })?;
    WorkflowPendingCommandUsecase::new(Arc::new(PendingWorkflowCommandFileRepository::new(
        data_dir.to_path_buf(),
    )))
    .enqueue_pending_command(PendingWorkflowCommand {
        command_id: command_id.clone(),
        execution_id: execution_id.to_string(),
        requested_at,
        payload,
    })
    .map_err(|error| CliError::Other(format!("Failed to enqueue pending command: {error}")))?;
    let path = data_dir
        .join("workflow_pending")
        .join("pending")
        .join(format!("{command_id}.json"));
    Ok(PendingEnqueueOutput {
        execution_id: execution_id.to_string(),
        request_id: command_id,
        path: path.display().to_string(),
    })
}

pub(super) fn ensure_execution_exists(data_dir: &Path, execution_id: &str) -> Result<(), CliError> {
    if get_execution_summary_file_direct(data_dir, execution_id)?.is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow execution not found: {execution_id}"
        )));
    }
    Ok(())
}

pub(super) fn get_execution_summary_file_direct(
    data_dir: &Path,
    execution_id: &str,
) -> Result<Option<WorkflowExecutionSummary>, CliError> {
    validate_execution_id(execution_id)?;
    file_direct_query_service(data_dir)
        .get_execution(execution_id)
        .map_err(|error| CliError::Other(error.to_string()))
}

/// Mutation preflight で必要な execution definition を読むための制限付き helper。
/// Read-only 公開 command は [`WorkflowQueryService`] を直接使う。
pub(super) fn read_execution_events(
    data_dir: &Path,
    execution_id: &str,
) -> Result<Vec<WorkflowEventDraft>, CliError> {
    let execution_id = crate::domain::workflow::WorkflowExecutionId::new(execution_id.to_string())
        .map_err(|error| CliError::InvalidInput(error.to_string()))?;
    WorkflowEventLogRepository::new(data_dir.to_path_buf())
        .read(&execution_id)
        .map_err(|error| CliError::Other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};

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
    }
}
