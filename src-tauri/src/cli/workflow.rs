use std::path::{Path, PathBuf};

use clap::Subcommand;

use super::api_client;
use super::common::{
    truncate, validate_execution_id, validate_node, validate_optional_cli_text_len, CliError,
};
use super::file_direct;
use super::output::OutputSubcommand;
use crate::adaptor::controller::api::protocol::{ApproveNodeRequest, StartExecutionRequest};
use crate::adaptor::protocol::workflow::{ExecutionStatusView, WorkflowExecutionView};
use crate::domain::workflow::ExecutionStatusFilter;
use crate::usecase::workflow::dto::{
    ExecutionStatusDto, WorkflowExecutionSummaryDto, WorkflowSummaryDto,
};

/// `releash workflow` の command / query 集合。
#[derive(Subcommand, Debug)]
pub(super) enum WorkflowSubcommand {
    /// 利用可能な workflow definition 一覧を表示する。
    List {
        #[arg(long)]
        json: bool,
    },
    /// workflow definition を名前で解決して execution を開始する。
    Start {
        workflow_name: String,
        /// workflow の request Artifact。省略時は空文字列。
        request: Option<String>,
        #[arg(long, value_name = "PATH")]
        worktree: Option<PathBuf>,
        #[arg(long, value_parser = ["ask", "edit", "full"])]
        permission: Option<String>,
    },
    /// 過去 / 進行中の workflow execution 一覧を表示する。
    Executions {
        /// status filter: `active` または `terminal`。
        #[arg(long)]
        status: Option<String>,
        /// worktree path filter。
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 指定 execution の現在 read model を表示する。
    Status {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// 指定 execution の event log を表示する。
    Logs {
        execution_id: String,
        #[arg(long)]
        json: bool,
    },
    /// approval checkpoint で待機中の NodeExecution を承認する。
    Approve {
        execution_id: String,
        #[arg(long)]
        node: String,
        #[arg(long = "node-execution", value_name = "NODE_EXECUTION_ID")]
        node_execution: Option<String>,
        #[arg(long)]
        comment: Option<String>,
    },
    /// 進行中の workflow execution を中止する。
    Abort { execution_id: String },
    /// 進行中の workflow execution を再開可能な checkpoint で停止する。
    Stop { execution_id: String },
    /// interrupted 状態の workflow execution を再開する。
    Resume { execution_id: String },
    /// node の Artifact に対する typed CLI 入口。
    Output {
        #[command(subcommand)]
        command: OutputSubcommand,
    },
}

pub(super) fn cmd_list(
    workflows_dir: &Path,
    data_dir: &Path,
    json: bool,
) -> Result<String, CliError> {
    let summaries = api_client::read_with_fallback(
        data_dir,
        |client| client.workflows(),
        || file_direct::list_workflows(workflows_dir, data_dir),
    )?;
    format_workflow_list(summaries, json)
}

pub(super) fn cmd_start(
    data_dir: &Path,
    workflow_name: String,
    request: Option<String>,
    worktree: Option<PathBuf>,
    permission: Option<String>,
) -> Result<String, CliError> {
    let request = build_start_request(workflow_name, request, worktree, permission, || {
        std::env::current_dir()
    })?;
    let response = api_client::mutation(data_dir, |client| client.start_workflow(&request))?;
    Ok(format!("started: execution_id={}\n", response.execution_id))
}

fn build_start_request(
    workflow_name: String,
    request: Option<String>,
    worktree: Option<PathBuf>,
    permission: Option<String>,
    current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<StartExecutionRequest, CliError> {
    if workflow_name.trim().is_empty() {
        return Err(CliError::InvalidInput(
            "workflow-name must not be empty".to_string(),
        ));
    }
    let worktree = match worktree {
        Some(path) => path,
        None => current_dir().map_err(|error| {
            CliError::Other(format!("current directory を取得できません: {error}"))
        })?,
    };
    Ok(StartExecutionRequest {
        workflow_name,
        worktree_path: worktree.to_string_lossy().into_owned(),
        request: request.unwrap_or_default(),
        permission_mode: permission,
        created_from: Some(api_client::start_created_from().to_string()),
    })
}

pub(super) fn cmd_executions(
    data_dir: &Path,
    status: Option<String>,
    worktree: Option<String>,
    json: bool,
) -> Result<String, CliError> {
    let status_filter = parse_status_filter(status.as_deref())?;
    let executions = api_client::read_with_fallback(
        data_dir,
        |client| client.executions(status.as_deref(), worktree.as_deref()),
        || file_direct::list_executions(data_dir, status_filter, worktree.as_deref()),
    )?;
    format_execution_list(executions, json)
}

pub(super) fn cmd_status(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    let execution = api_client::read_with_fallback(
        data_dir,
        |client| client.execution_status(execution_id),
        || file_direct::execution_status(data_dir, execution_id),
    )?;
    format_execution_status(execution, json)
}

pub(super) fn cmd_logs(
    data_dir: &Path,
    execution_id: &str,
    json: bool,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    let events = api_client::read_with_fallback(
        data_dir,
        |client| client.execution_log(execution_id),
        || file_direct::execution_log(data_dir, execution_id),
    )?;
    format_execution_log(events, json)
}

pub(super) fn cmd_approve(
    data_dir: &Path,
    execution_id: &str,
    node: String,
    node_execution: Option<String>,
    comment: Option<String>,
) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    validate_node(&node)?;
    validate_optional_cli_text_len(comment.as_deref(), "--comment")?;
    let request = ApproveNodeRequest {
        node: node.clone(),
        node_execution_id: api_client::resolve_node_execution_id(node_execution),
        comment,
    };
    api_client::mutation(data_dir, |client| client.approve(execution_id, &request))?;
    Ok(format!(
        "approved: execution_id={execution_id} node={node}\n"
    ))
}

pub(super) fn cmd_abort(data_dir: &Path, execution_id: &str) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    api_client::mutation(data_dir, |client| client.abort(execution_id))?;
    Ok(format!("aborted: execution_id={execution_id}\n"))
}

pub(super) fn cmd_stop(data_dir: &Path, execution_id: &str) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    api_client::mutation(data_dir, |client| client.stop(execution_id))?;
    Ok(format!("stopped: execution_id={execution_id}\n"))
}

pub(super) fn cmd_resume(data_dir: &Path, execution_id: &str) -> Result<String, CliError> {
    validate_execution_id(execution_id)?;
    api_client::mutation(data_dir, |client| client.resume(execution_id))?;
    Ok(format!("resumed: execution_id={execution_id}\n"))
}

fn format_workflow_list(
    summaries: Vec<WorkflowSummaryDto>,
    json: bool,
) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&summaries)
            .map_err(|error| CliError::Other(format!("serialize workflows: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    if summaries.is_empty() {
        return Ok("(no workflows)\n".to_string());
    }
    let mut output = String::new();
    for summary in summaries {
        let tag = if summary.builtin {
            "[builtin]"
        } else {
            "         "
        };
        let active = if summary.is_running { " (active)" } else { "" };
        output.push_str(&format!(
            "{tag} {:<32}  {}{active}\n",
            summary.name, summary.description
        ));
    }
    Ok(output)
}

fn format_execution_list(
    executions: Vec<WorkflowExecutionSummaryDto>,
    json: bool,
) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&executions)
            .map_err(|error| CliError::Other(format!("serialize executions: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    if executions.is_empty() {
        return Ok("(no executions)\n".to_string());
    }
    let mut output = format!(
        "{:<36}  {:<20}  {:<18}  WORKTREE\n",
        "EXECUTION_ID", "WORKFLOW", "STATUS"
    );
    for execution in executions {
        output.push_str(&format!(
            "{:<36}  {:<20}  {:<18}  {}\n",
            execution.execution_id,
            truncate(&execution.workflow_name, 20),
            execution_status_dto_name(execution.status),
            execution.worktree_path
        ));
    }
    Ok(output)
}

fn format_execution_status(
    execution: WorkflowExecutionView,
    json: bool,
) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&execution)
            .map_err(|error| CliError::Other(format!("serialize execution: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let current_node = execution.current_node.as_deref().unwrap_or("");
    Ok(format!(
        "execution_id:  {}\nworkflow:      {}\nstatus:        {}\ncurrent_node:  {}\nupdated_at:    {}\ninput_tokens:  {}\noutput_tokens: {}\n",
        execution.id,
        execution.workflow_name,
        execution_status_name(execution.status),
        current_node,
        execution.updated_at,
        execution.total_token_usage.input_tokens,
        execution.total_token_usage.output_tokens,
    ))
}

fn format_execution_log(events: Vec<serde_json::Value>, json: bool) -> Result<String, CliError> {
    if json {
        let text = serde_json::to_string_pretty(&events)
            .map_err(|error| CliError::Other(format!("serialize log: {error}")))?;
        return Ok(format!("{text}\n"));
    }
    let mut output = String::new();
    for event in events {
        let kind = event
            .get("event")
            .and_then(serde_json::Value::as_str)
            .map(event_kind_display_name)
            .unwrap_or_else(|| "MalformedEventLogRecord".to_string());
        let json = serde_json::to_string(&event)
            .map_err(|error| CliError::Other(format!("serialize log event: {error}")))?;
        output.push_str(&format!("{kind} {json}\n"));
    }
    Ok(output)
}

fn parse_status_filter(value: Option<&str>) -> Result<Option<ExecutionStatusFilter>, CliError> {
    ExecutionStatusFilter::from_public_filter(value).map_err(|_| {
        CliError::InvalidInput(format!(
            "Invalid --status value: {} (expected: active | terminal)",
            value.unwrap_or_default()
        ))
    })
}

fn execution_status_name(status: ExecutionStatusView) -> &'static str {
    match status {
        ExecutionStatusView::Running => "running",
        ExecutionStatusView::WaitingApproval => "waiting_approval",
        ExecutionStatusView::Completed => "completed",
        ExecutionStatusView::Failed => "failed",
        ExecutionStatusView::Aborted => "aborted",
        ExecutionStatusView::Interrupted => "interrupted",
    }
}

fn execution_status_dto_name(status: ExecutionStatusDto) -> &'static str {
    match status {
        ExecutionStatusDto::Running => "running",
        ExecutionStatusDto::WaitingApproval => "waiting_approval",
        ExecutionStatusDto::Completed => "completed",
        ExecutionStatusDto::Failed => "failed",
        ExecutionStatusDto::Aborted => "aborted",
        ExecutionStatusDto::Interrupted => "interrupted",
    }
}

fn event_kind_display_name(kind: &str) -> String {
    kind.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::common::test_support::{
        append_workflow_event, execution_started_event, initialize_canonical_store, make_execution,
        test_uuid, write_execution_file,
    };
    use super::super::Cli;
    use super::*;
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};
    use clap::Parser;
    use tempfile::TempDir;

    fn seed_execution(data_dir: &Path, execution_id: &str, status: ExecutionStatus) {
        write_execution_file(
            data_dir,
            &make_execution(execution_id, "/repo", status, 100.0),
        );
        append_workflow_event(
            data_dir,
            &execution_started_event(execution_id, "wf", "/repo"),
        );
    }

    #[test]
    fn execution_commands_parse_with_canonical_vocabulary() {
        let execution_id = "550e8400-e29b-41d4-a716-446655440000";
        for argv in [
            vec!["releash", "workflow", "start", "review"],
            vec![
                "releash",
                "workflow",
                "start",
                "review",
                "please review",
                "--permission",
                "edit",
            ],
            vec!["releash", "workflow", "executions"],
            vec!["releash", "workflow", "executions", "--status", "active"],
            vec!["releash", "workflow", "status", execution_id, "--json"],
            vec!["releash", "workflow", "logs", execution_id],
            vec![
                "releash",
                "workflow",
                "approve",
                execution_id,
                "--node",
                "review",
            ],
            vec!["releash", "workflow", "abort", execution_id],
            vec!["releash", "workflow", "stop", execution_id],
            vec!["releash", "workflow", "resume", execution_id],
        ] {
            assert!(Cli::try_parse_from(argv).is_ok());
        }
    }

    #[test]
    fn malformed_log_record_uses_a_neutral_display_label() {
        let output =
            format_execution_log(vec![serde_json::json!({"execution_id": "broken"})], false)
                .unwrap();

        assert!(output.starts_with("MalformedEventLogRecord "));
        assert!(!output.contains(&["Workflow", "Event"].concat()));
    }

    #[test]
    fn start_defaults_request_worktree_and_origin_at_the_cli_boundary() {
        let _lock = TEST_ENV_LOCK.lock();
        let _node_execution = EnvVarGuard::set_value("RELEASH_NODE_EXECUTION_ID", "");
        let request = build_start_request("review".to_string(), None, None, None, || {
            Ok(PathBuf::from("/current/worktree"))
        })
        .unwrap();

        assert_eq!(request.workflow_name, "review");
        assert_eq!(request.request, "");
        assert_eq!(request.worktree_path, "/current/worktree");
        assert_eq!(request.permission_mode, None);
        assert_eq!(request.created_from.as_deref(), Some("cli"));
    }

    #[test]
    fn legacy_cli_vocabulary_is_rejected() {
        let execution_id = "550e8400-e29b-41d4-a716-446655440000";
        let legacy_collection = ["ru", "ns"].concat();
        let legacy_node_flag = ["--st", "ep"].concat();
        let legacy_rejection = ["Re", "ject"].concat();
        for argv in [
            vec![
                "releash".to_string(),
                "workflow".to_string(),
                legacy_collection,
            ],
            vec![
                "releash".to_string(),
                "workflow".to_string(),
                "approve".to_string(),
                execution_id.to_string(),
                legacy_node_flag,
                "review".to_string(),
            ],
            vec![
                "releash".to_string(),
                "workflow".to_string(),
                "abort".to_string(),
                execution_id.to_string(),
                "--node".to_string(),
                "review".to_string(),
            ],
            vec![
                "releash".to_string(),
                "workflow".to_string(),
                legacy_rejection,
                execution_id.to_string(),
            ],
            vec![
                "releash".to_string(),
                "task".to_string(),
                "list".to_string(),
            ],
        ] {
            assert!(
                Cli::try_parse_from(argv.clone()).is_err(),
                "legacy argv: {argv:?}"
            );
        }
    }

    #[test]
    fn workflow_help_contains_only_canonical_external_vocabulary() {
        let help = super::super::render_long_help();
        for required in [
            "start",
            "executions",
            "stop",
            "resume",
            "EXECUTION_ID",
            "--node",
        ] {
            assert!(help.contains(required), "missing help term: {required}");
        }
        for forbidden in [
            ["ru", "ns"].concat(),
            ["run", "_id"].concat(),
            ["--st", "ep"].concat(),
            ["Re", "ject"].concat(),
        ] {
            assert!(
                !help.contains(&forbidden),
                "forbidden help term: {forbidden}"
            );
        }
    }

    #[test]
    fn execution_list_and_status_use_file_fallback_when_app_is_not_running() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(1);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let listed = cmd_executions(temp.path(), None, None, true).unwrap();
        let listed: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed[0]["executionId"], execution_id);
        assert_eq!(listed[0]["status"], "running");

        let status = cmd_status(temp.path(), &execution_id, true).unwrap();
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["id"], execution_id);
        assert_eq!(status["status"], "running");
        assert_eq!(status["artifacts"][0]["nodeName"], "request");
    }

    #[test]
    fn log_output_uses_execution_event_schema_in_file_fallback() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(2);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let json = cmd_logs(temp.path(), &execution_id, true).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json[0]["event"], "execution_started");
        assert_eq!(json[0]["execution_id"], execution_id);

        let human = cmd_logs(temp.path(), &execution_id, false).unwrap();
        assert!(human.starts_with("ExecutionStarted "));
    }

    #[test]
    fn file_fallback_and_tauri_surfaces_read_the_same_projection() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(3);
        seed_execution(temp.path(), &execution_id, ExecutionStatus::Running);

        let cli = file_direct::execution_status(temp.path(), &execution_id).unwrap();
        let tauri = crate::adaptor::controller::wiring::build_workflow_usecase(temp.path())
            .get_execution_state(&execution_id)
            .unwrap()
            .map(crate::adaptor::presenter::workflow::workflow_execution_to_view)
            .unwrap();
        assert_eq!(cli, tauri);
    }

    #[test]
    fn execution_summary_json_matches_backend_dto_shape() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(4);
        let mut execution = make_execution(
            &execution_id,
            "/repo",
            ExecutionStatus::WaitingApproval,
            5.0,
        );
        execution.current_node = Some("review".to_string());
        execution.created_from = ExecutionOrigin::Agent;
        execution.total_token_usage = TokenUsage {
            input_tokens: 8,
            output_tokens: 3,
        };
        write_execution_file(temp.path(), &execution);

        let actual: serde_json::Value =
            serde_json::from_str(&cmd_executions(temp.path(), None, None, true).unwrap()).unwrap();
        assert_eq!(actual[0]["executionId"], execution_id);
        assert_eq!(actual[0]["createdFrom"], "agent");
        assert_eq!(actual[0]["totalTokenUsage"]["inputTokens"], 8);
    }

    #[test]
    fn missing_execution_is_reported_without_creating_state() {
        let temp = TempDir::new().unwrap();
        initialize_canonical_store(temp.path());
        let execution_id = test_uuid(5);
        let error = cmd_status(temp.path(), &execution_id, false).unwrap_err();
        assert_eq!(
            error,
            CliError::NotFound(format!("Workflow execution not found: {execution_id}"))
        );
    }

    #[test]
    fn invalid_filter_is_rejected() {
        let temp = TempDir::new().unwrap();
        assert!(matches!(
            cmd_executions(temp.path(), Some("paused".to_string()), None, false),
            Err(CliError::InvalidInput(_))
        ));
    }

    #[test]
    fn stop_and_resume_reject_invalid_execution_ids_before_api_discovery() {
        let temp = TempDir::new().unwrap();
        assert!(matches!(
            cmd_stop(temp.path(), "not-a-uuid"),
            Err(CliError::InvalidInput(_))
        ));
        assert!(matches!(
            cmd_resume(temp.path(), "not-a-uuid"),
            Err(CliError::InvalidInput(_))
        ));
    }

    #[test]
    fn stop_and_resume_require_the_local_api() {
        let temp = TempDir::new().unwrap();
        let execution_id = test_uuid(6);
        for result in [
            cmd_stop(temp.path(), &execution_id),
            cmd_resume(temp.path(), &execution_id),
        ] {
            assert!(
                matches!(result, Err(CliError::Other(message)) if message.contains("アプリの起動が必要"))
            );
        }
    }
}
