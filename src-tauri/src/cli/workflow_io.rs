use std::path::Path;

use super::common::{validate_run_id, CliError};
use crate::adaptor::gateway::workflow::{
    PendingWorkflowCommandFileRepository, WorkflowEventLogRepository, WorkflowRunFileRepository,
};
use crate::domain::workflow::{RunId, WorkflowRunRepository, WorkflowRunSummary};
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::workflow::command::WorkflowPendingCommandUsecase;
use crate::usecase::workflow::ports::{
    PendingWorkflowCommand, WorkflowEventDraft, WorkflowEventRepository,
};

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(super) enum CliRequestPayload {
    Approve {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        comment: Option<String>,
    },
    Reject {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
        reason: String,
    },
    Abort {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        node_name: Option<String>,
    },
    SubmitOutput {
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingEnqueueOutput {
    pub(super) run_id: String,
    pub(super) request_id: String,
    pub(super) path: String,
}

impl PendingEnqueueOutput {
    pub(super) fn format_stdout_line(&self) -> String {
        format!(
            "queued: run_id={} request_id={} ({})",
            self.run_id, self.request_id, self.path
        )
    }
}

pub(super) fn enqueue_pending_command(
    data_dir: &Path,
    run_id: &str,
    payload: CliRequestPayload,
) -> Result<PendingEnqueueOutput, CliError> {
    validate_run_id(run_id)?;
    // [06] 入口バリデーション (5-2 修正): 不在 run_id への mutation は engine で
    // silent-drop されるため、pending file 書き出し前に CLI で弾く。spec [06] の
    // 「CLI 完了基準＝pending file 書き出しまで」境界は維持され、本チェックは
    // 書き出し前の入口バリデーションとして位置づける。
    ensure_run_exists(data_dir, run_id)?;
    let command_id = uuid::Uuid::new_v4().to_string();
    let requested_at = current_timestamp();
    let payload = serde_json::to_value(payload)
        .map_err(|e| CliError::Other(format!("Failed to serialize pending command: {e}")))?;
    WorkflowPendingCommandUsecase::new(std::sync::Arc::new(
        PendingWorkflowCommandFileRepository::new(data_dir.to_path_buf()),
    ))
    .enqueue_pending_command(PendingWorkflowCommand {
        command_id: command_id.clone(),
        run_id: run_id.to_string(),
        requested_at,
        payload,
    })
    .map_err(|e| CliError::Other(format!("Failed to enqueue pending command: {e}")))?;
    let path = data_dir
        .join("workflow_pending")
        .join("pending")
        .join(format!("{command_id}.json"));
    Ok(PendingEnqueueOutput {
        run_id: run_id.to_string(),
        request_id: command_id,
        path: path.display().to_string(),
    })
}

pub(super) fn ensure_run_exists(data_dir: &Path, run_id: &str) -> Result<(), CliError> {
    if get_run_summary_file_direct(data_dir, run_id).is_none() {
        return Err(CliError::NotFound(format!(
            "Workflow run not found: {run_id}"
        )));
    }
    Ok(())
}

pub(super) fn get_run_summary_file_direct(
    data_dir: &Path,
    run_id: &str,
) -> Option<WorkflowRunSummary> {
    let run_id = RunId::new(run_id.to_string()).ok()?;
    WorkflowRunFileRepository::new(data_dir.to_path_buf())
        .get_run(&run_id)
        .ok()
        .flatten()
}

pub(super) fn read_domain_log(
    data_dir: &Path,
    run_id: &str,
) -> Result<Vec<WorkflowEventDraft>, CliError> {
    let run_id_value =
        RunId::new(run_id.to_string()).map_err(|e| CliError::InvalidInput(e.to_string()))?;
    WorkflowEventLogRepository::new(data_dir.to_path_buf())
        .read(&run_id_value)
        .map_err(|e| CliError::Other(e.to_string()))
}
